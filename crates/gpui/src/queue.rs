//! A queue with three priority levels. A sender can be shared by reference across
//! threads, while cloning a receiver adds another consumer of the same queue.
//!
//! When more than one level has items, each pop chooses a level at random in proportion to its
//! [`Priority::weight`]: high 60, medium 30 and low 10. Items retain FIFO order within a level;
//! the queue itself is unbounded.

use std::{
    collections::VecDeque,
    fmt,
    iter::FusedIterator,
    sync::{Arc, atomic::AtomicUsize},
};

use rand::{Rng, SeedableRng, rngs::SmallRng};

use crate::Priority;

struct PriorityQueues<T> {
    high_priority: VecDeque<T>,
    medium_priority: VecDeque<T>,
    low_priority: VecDeque<T>,
}

impl<T> PriorityQueues<T> {
    fn is_empty(&self) -> bool {
        self.high_priority.is_empty()
            && self.medium_priority.is_empty()
            && self.low_priority.is_empty()
    }
}

struct PriorityQueueState<T> {
    queues: parking_lot::Mutex<PriorityQueues<T>>,
    condvar: parking_lot::Condvar,
    receiver_count: AtomicUsize,
    sender_count: AtomicUsize,
}

impl<T> PriorityQueueState<T> {
    fn send(&self, priority: Priority, item: T) -> Result<(), SendError<T>> {
        if self
            .receiver_count
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return Err(SendError(item));
        }

        let mut queues = self.queues.lock();
        Self::push(&mut queues, priority, item);
        self.condvar.notify_one();
        Ok(())
    }

    fn spin_send(&self, priority: Priority, item: T) -> Result<(), SendError<T>> {
        if self
            .receiver_count
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return Err(SendError(item));
        }

        let mut queues = loop {
            if let Some(guard) = self.queues.try_lock() {
                break guard;
            }
            std::hint::spin_loop();
        };
        Self::push(&mut queues, priority, item);
        self.condvar.notify_one();
        Ok(())
    }

    fn push(queues: &mut PriorityQueues<T>, priority: Priority, item: T) {
        match priority {
            Priority::RealtimeAudio => unreachable!(
                "Realtime audio priority runs on a dedicated thread and is never queued"
            ),
            Priority::High => queues.high_priority.push_back(item),
            Priority::Medium => queues.medium_priority.push_back(item),
            Priority::Low => queues.low_priority.push_back(item),
        };
    }

    fn recv<'a>(&'a self) -> Result<parking_lot::MutexGuard<'a, PriorityQueues<T>>, RecvError> {
        let mut queues = self.queues.lock();

        // Re-check on every wakeup: the last sender dropping is the only thing that ends the wait.
        while queues.is_empty() {
            let sender_count = self.sender_count.load(std::sync::atomic::Ordering::Relaxed);
            if sender_count == 0 {
                return Err(crate::queue::RecvError);
            }

            self.condvar.wait(&mut queues);
        }

        Ok(queues)
    }

    fn try_recv<'a>(
        &'a self,
    ) -> Result<Option<parking_lot::MutexGuard<'a, PriorityQueues<T>>>, RecvError> {
        let mut queues = self.queues.lock();

        let sender_count = self.sender_count.load(std::sync::atomic::Ordering::Relaxed);
        if queues.is_empty() && sender_count == 0 {
            return Err(crate::queue::RecvError);
        }

        if queues.is_empty() {
            Ok(None)
        } else {
            Ok(Some(queues))
        }
    }

    fn spin_try_recv<'a>(
        &'a self,
    ) -> Result<Option<parking_lot::MutexGuard<'a, PriorityQueues<T>>>, RecvError> {
        let queues = loop {
            if let Some(guard) = self.queues.try_lock() {
                break guard;
            }
            std::hint::spin_loop();
        };

        let sender_count = self.sender_count.load(std::sync::atomic::Ordering::Relaxed);
        if queues.is_empty() && sender_count == 0 {
            return Err(crate::queue::RecvError);
        }

        if queues.is_empty() {
            Ok(None)
        } else {
            Ok(Some(queues))
        }
    }
}

/// The sending half of a queue made by [`PriorityQueueReceiver::new`].
///
/// This type isn't `Clone`. Once the sender is dropped, a receiver finishes after it
/// drains the queue.
#[doc(hidden)]
pub struct PriorityQueueSender<T> {
    state: Arc<PriorityQueueState<T>>,
}

impl<T> PriorityQueueSender<T> {
    fn new(state: Arc<PriorityQueueState<T>>) -> Self {
        Self { state }
    }

    /// Adds `item` at the back of the given priority's queue and wakes a waiting receiver.
    ///
    /// # Errors
    ///
    /// Returns [`SendError`] with the item if every receiver has been dropped.
    /// Panics if `priority` is [`Priority::RealtimeAudio`], which never goes through this queue.
    pub fn send(&self, priority: Priority, item: T) -> Result<(), SendError<T>> {
        self.state.send(priority, item)?;
        Ok(())
    }

    /// Like [`Self::send`], but spins until the queue's lock is free instead of blocking
    /// the thread on it.
    ///
    /// # Errors
    ///
    /// Returns [`SendError`] with the item if every receiver has been dropped.
    pub fn spin_send(&self, priority: Priority, item: T) -> Result<(), SendError<T>> {
        self.state.spin_send(priority, item)?;
        Ok(())
    }
}

impl<T> Drop for PriorityQueueSender<T> {
    fn drop(&mut self) {
        // Hold the lock so a receiver can't check the count and then miss this wakeup.
        let _queues = self.state.queues.lock();
        if self
            .state
            .sender_count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            == 1
        {
            self.state.condvar.notify_all();
        }
    }
}

/// The receiving half of a priority queue. It pops items in weighted random order across
/// the three priority levels, while preserving FIFO order within each level.
///
/// Cloning a receiver adds another consumer to the same queue.
#[doc(hidden)]
pub struct PriorityQueueReceiver<T> {
    state: Arc<PriorityQueueState<T>>,
    rand: SmallRng,
    disconnected: bool,
}

impl<T> Clone for PriorityQueueReceiver<T> {
    fn clone(&self) -> Self {
        self.state
            .receiver_count
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            state: Arc::clone(&self.state),
            rand: SmallRng::seed_from_u64(0),
            disconnected: self.disconnected,
        }
    }
}

/// Returned by a send when no receiver is left. Holds the item that wasn't sent.
#[doc(hidden)]
pub struct SendError<T>(pub T);

impl<T: fmt::Debug> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SendError").field(&self.0).finish()
    }
}

/// Returned by a receive when the queue is empty and its sender has been dropped.
#[derive(Debug)]
#[doc(hidden)]
pub struct RecvError;

#[allow(dead_code)]
impl<T> PriorityQueueReceiver<T> {
    /// Creates an empty queue and returns its sender and receiver.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::{Priority, queue::PriorityQueueReceiver};
    /// let (sender, mut receiver) = PriorityQueueReceiver::new();
    /// sender.send(Priority::High, 1).unwrap();
    /// assert_eq!(receiver.pop().unwrap(), 1);
    ///
    /// // Once the sender is dropped and the queue is empty, `pop` fails.
    /// drop(sender);
    /// assert!(receiver.pop().is_err());
    /// ```
    pub fn new() -> (PriorityQueueSender<T>, Self) {
        let state = PriorityQueueState {
            queues: parking_lot::Mutex::new(PriorityQueues {
                high_priority: VecDeque::new(),
                medium_priority: VecDeque::new(),
                low_priority: VecDeque::new(),
            }),
            condvar: parking_lot::Condvar::new(),
            receiver_count: AtomicUsize::new(1),
            sender_count: AtomicUsize::new(1),
        };
        let state = Arc::new(state);

        let sender = PriorityQueueSender::new(Arc::clone(&state));

        let receiver = PriorityQueueReceiver {
            state,
            rand: SmallRng::seed_from_u64(0),
            disconnected: false,
        };

        (sender, receiver)
    }

    /// Returns `true` if no items are queued at any priority.
    pub fn is_empty(&self) -> bool {
        self.state.queues.lock().is_empty()
    }

    /// Returns the number of queued items across all priorities.
    pub(crate) fn len(&self) -> usize {
        let queues = self.state.queues.lock();
        queues.high_priority.len() + queues.medium_priority.len() + queues.low_priority.len()
    }

    /// Pops one item without blocking. Returns `Ok(None)` if the queue is empty.
    ///
    /// Use this to take a single item. To drain many items, use [`Self::try_iter`].
    ///
    /// # Errors
    ///
    /// Returns [`RecvError`] when the queue is empty and its sender has been dropped.
    pub fn try_pop(&mut self) -> Result<Option<T>, RecvError> {
        self.pop_inner(false)
    }

    /// Like [`Self::try_pop`], but spins until the queue's lock is free instead of
    /// blocking the thread on it.
    ///
    /// # Errors
    ///
    /// Returns [`RecvError`] when the queue is empty and its sender has been dropped.
    pub fn spin_try_pop(&mut self) -> Result<Option<T>, RecvError> {
        use Priority as P;

        let Some(mut queues) = self.state.spin_try_recv()? else {
            return Ok(None);
        };

        let high = P::High.weight() * !queues.high_priority.is_empty() as u32;
        let medium = P::Medium.weight() * !queues.medium_priority.is_empty() as u32;
        let low = P::Low.weight() * !queues.low_priority.is_empty() as u32;
        let mut mass = high + medium + low;

        if !queues.high_priority.is_empty() {
            let flip = self.rand.random_ratio(P::High.weight(), mass);
            if flip {
                return Ok(queues.high_priority.pop_front());
            }
            mass -= P::High.weight();
        }

        if !queues.medium_priority.is_empty() {
            let flip = self.rand.random_ratio(P::Medium.weight(), mass);
            if flip {
                return Ok(queues.medium_priority.pop_front());
            }
            mass -= P::Medium.weight();
        }

        if !queues.low_priority.is_empty() {
            let flip = self.rand.random_ratio(P::Low.weight(), mass);
            if flip {
                return Ok(queues.low_priority.pop_front());
            }
        }

        Ok(None)
    }

    /// Pops one item, blocking the current thread until one is available.
    ///
    /// Use this to take a single item. To drain many items, use [`Self::iter`].
    ///
    /// # Errors
    ///
    /// Returns [`RecvError`] when the queue is empty and its sender has been dropped.
    pub fn pop(&mut self) -> Result<T, RecvError> {
        self.pop_inner(true).map(|e| e.unwrap())
    }

    /// Returns an iterator that yields the queued items and ends when the queue is empty.
    ///
    /// The iterator never blocks. Its last item is `Err(RecvError)` if the sender has
    /// been dropped, and `None` otherwise.
    pub fn try_iter(self) -> TryIter<T> {
        TryIter {
            receiver: self,
            ended: false,
        }
    }

    /// Returns an iterator that yields items as they arrive, in the same weighted order as
    /// [`Self::pop`].
    ///
    /// The iterator blocks while the queue is empty and ends once the sender has been
    /// dropped and the queue is drained.
    pub fn iter(self) -> Iter<T> {
        Iter(self)
    }

    #[inline(always)]
    // algorithm is the loaded die from biased coin from
    // https://www.keithschwarz.com/darts-dice-coins/
    fn pop_inner(&mut self, block: bool) -> Result<Option<T>, RecvError> {
        use Priority as P;

        let mut queues = if !block {
            let Some(queues) = self.state.try_recv()? else {
                return Ok(None);
            };
            queues
        } else {
            self.state.recv()?
        };

        let high = P::High.weight() * !queues.high_priority.is_empty() as u32;
        let medium = P::Medium.weight() * !queues.medium_priority.is_empty() as u32;
        let low = P::Low.weight() * !queues.low_priority.is_empty() as u32;
        let mut mass = high + medium + low; //%

        if !queues.high_priority.is_empty() {
            let flip = self.rand.random_ratio(P::High.weight(), mass);
            if flip {
                return Ok(queues.high_priority.pop_front());
            }
            mass -= P::High.weight();
        }

        if !queues.medium_priority.is_empty() {
            let flip = self.rand.random_ratio(P::Medium.weight(), mass);
            if flip {
                return Ok(queues.medium_priority.pop_front());
            }
            mass -= P::Medium.weight();
        }

        if !queues.low_priority.is_empty() {
            let flip = self.rand.random_ratio(P::Low.weight(), mass);
            if flip {
                return Ok(queues.low_priority.pop_front());
            }
        }

        Ok(None)
    }
}

impl<T> Drop for PriorityQueueReceiver<T> {
    fn drop(&mut self) {
        self.state
            .receiver_count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The blocking iterator returned by [`PriorityQueueReceiver::iter`].
#[doc(hidden)]
pub struct Iter<T>(PriorityQueueReceiver<T>);
impl<T> Iterator for Iter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop().ok()
    }
}
impl<T> FusedIterator for Iter<T> {}

/// The non-blocking iterator returned by [`PriorityQueueReceiver::try_iter`].
#[doc(hidden)]
pub struct TryIter<T> {
    receiver: PriorityQueueReceiver<T>,
    ended: bool,
}
impl<T> Iterator for TryIter<T> {
    type Item = Result<T, RecvError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }

        let res = self.receiver.try_pop();
        self.ended = !matches!(&res, Ok(Some(_)));

        res.transpose()
    }
}
impl<T> FusedIterator for TryIter<T> {}

#[cfg(test)]
mod tests {
    use collections::HashSet;

    use super::*;

    #[test]
    fn dropping_the_last_sender_wakes_a_waiting_receiver() {
        let (sender, mut receiver) = PriorityQueueReceiver::<usize>::new();
        let (finished, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            finished.send(receiver.pop().is_err()).ok();
        });

        // Give the receiver time to block on the empty queue.
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(sender);
        assert_eq!(
            result.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(true)
        );
    }

    #[test]
    fn nonblocking_iterator_stays_finished_after_observing_an_empty_queue() {
        let (sender, receiver) = PriorityQueueReceiver::new();
        sender.send(Priority::Medium, 1).unwrap();
        let mut items = receiver.try_iter();
        assert!(matches!(items.next(), Some(Ok(1))));
        assert!(items.next().is_none());
        sender.send(Priority::High, 2).unwrap();
        assert!(items.next().is_none());
    }

    #[test]
    fn all_tasks_get_yielded() {
        let (tx, mut rx) = PriorityQueueReceiver::new();
        tx.send(Priority::Medium, 20).unwrap();
        tx.send(Priority::High, 30).unwrap();
        tx.send(Priority::Low, 10).unwrap();
        tx.send(Priority::Medium, 21).unwrap();
        tx.send(Priority::High, 31).unwrap();

        drop(tx);

        assert_eq!(
            rx.iter().collect::<HashSet<_>>(),
            [30, 31, 20, 21, 10].into_iter().collect::<HashSet<_>>()
        )
    }

    #[test]
    fn new_high_prio_task_get_scheduled_quickly() {
        let (tx, mut rx) = PriorityQueueReceiver::new();
        for _ in 0..100 {
            tx.send(Priority::Low, 1).unwrap();
        }

        assert_eq!(rx.pop().unwrap(), 1);
        tx.send(Priority::High, 3).unwrap();
        assert_eq!(rx.pop().unwrap(), 3);
        assert_eq!(rx.pop().unwrap(), 1);
    }
}
