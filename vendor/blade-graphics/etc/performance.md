# Performance

Blade doesn't expect to be faster than wgpu-hal, but it's important to understand how much the difference is. Testing is done on "bunnymark" example, which is ported from wgpu. Since every draw call is dynamic in Blade, this benchmark is the worst case of the usage.

## MacBook Pro 2016

GPU: Intel Iris Graphics 550

Metal:
  - Blade starts to slow down after about 23K bunnies
  - wgpu-hal starts at 60K bunnies
  - wgpu starts at 15K bunnies

Vulkan Portability:
  - Blade starts to slow down at around 18K bunnies

## Thinkpad T495s

GPU: Ryzen 3500U

Windows/Vulkan:
  - Blade starts at around 18K bunnies
  - wgpu-hal starts at 60K bunnies
  - wgpu starts at 20K bunnies

## Thinkpad Z13 gen1

GPU: Ryzen 6850U

Windows/Vulkan:
  - Blade starts at around 50K bunnies
  - wgpu-hal starts at 50K bunnies (also GPU-limited)
  - wgpu starts at around 15K bunnies

## Conclusions

Amazingly, Blade performance in the worst case scenario is on par with wgpu (but still far from wgpu-hal). This is the best outcome we could hope for.

As expected, Vulkan path on macOS via MoltenVK is slower than the native Metal backend.

Ergonomically, our example is 335 LOC versus 830 LOC of wgpu-hal and 370-750 LOC in wgpu (depending on how we count the example framework).

It's also closer to the hardware (than even wgpu-hal) and easier to debug.
