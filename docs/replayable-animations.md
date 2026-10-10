# Replayable element animations

`AnimationElement::replay_on(event_id)` restarts an existing animation when an
application event key changes, while preserving the element and its children.
It works with `with_animation` and `with_animations` and their existing `Motion`
passes, directions, easing and completion semantics.

```rust
use gpui::{prelude::*, *};

let notification_event = 1usize; // Change for each received application event.
let row = div()
    .child("Build finished successfully")
    .with_animation(
        "notification-animation", // Keep this ID stable across events.
        Animation::new(Motion::new(millis(180)).iterations(2).alternate()),
        |row, emphasis| row.bg(rgb(0x202b3d).lerp(&rgb(0x4875b0), emphasis)),
    )
    .replay_on(notification_event);
```

The motion showcase mounts a notification on its first event. Further events
replay its background and border pulse with the same payload and retained
controls. Its pointer/keyboard action opens activity details. Tab/Shift+Tab move
between notification controls, and Enter/Space activate the focused button.
The reduced-motion toggle applies the existing application preference. Preview
scenes rendered with it enabled settle and remain idle when it is disabled. Run
`cargo run -p gpui-ce --example motion_showcase --locked`.

## Event keys

First mount starts the animation normally. An unchanged key leaves its elapsed
clock unchanged; with the same animation description, active and completed runs
stay where they are. A changed key restarts from the configured origin and
initial delay, including during an active run. Replay does not retarget smoothly
from the displayed value; use existing transitions or springs for that behavior.

Keep the animation element ID and child IDs stable. The event key does not
become part of the child ID path. Keys are compared between renders, so events
coalesced before a render produce one replay of the latest key, rather than an
event queue. Removing `replay_on` returns to ordinary sampling at the retained
elapsed time.

Enabling reduced motion resolves an already-observed replay key to its
configured resting value and consumes the run, even if the preference is
disabled before the next layout. Disabling the preference does not resume that
run. A new mount or new key observed after disabling it starts normally; earlier
preference changes do not consume that new run. Throttled timer work is
cancelled. Frame callbacks already queued before the preference change may drain
once.

`repeat_synced` preserves the App epoch. Replay restarts sequence placement, but
synchronized values and single-animation completion still use that epoch. Use an
ordinary animation when each event needs its own phase. Raw `Motion` sampling
remains independent of reduced-motion policy.

## Sequential timing

Replaying `with_animations` restarts the whole chain. Placement includes each
animation's initial delay and every configured pass. Late frames carry remaining
elapsed time into the selected animation and may skip completed steps. Only the
selected animation's callback runs.

An exact handoff samples the incoming ordinary animation at local elapsed time
zero. Reverse motion or custom easing can make its presented progress nonzero.
A synchronized incoming animation still samples its value at the App epoch.

Empty chains leave the element unchanged. Chains with zero total duration settle
in one render. The last reachable animation holds its actual terminal sample,
including custom endpoints. An unbounded animation or overflowing duration makes
later animations unreachable.

Frame throttling follows the selected animation. Changing its FPS cap replaces
an obsolete pending timer without replaying the run. Replay, handoff, completion
and disposal also cancel obsolete timer work.
