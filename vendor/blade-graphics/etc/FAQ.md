# Frequency Asked Questions

## When should I *not* use Blade?

- When you *target the Web*. Blade currently has no Web backends supported. Targeting WebGPU is desired, but will not be as performant as native.
- Similarly, when you target the *low-end GPUs* or old drivers. Blade has no OpenGL/D3D11 support, and it requires fresh drivers on Vulkan.
- When you render with 10K or *more draw calls*. State switching has overhead with Blade, and it is lower in GPU abstractions/libraries that have barriers and explicit bind groups.
- When you need something *off the shelf*. Blade is experimental and young, it assumes you'll be customizing it.

## Why investing into this when there is `wgpu`?

`wgpu` is becoming a standard solution for GPU access in Rust and beyond. It's wonderful, and by any means just use it if you have any doubts. It's a strong local maxima in a chosen space of low-level portability. It may very well be the global maxima as well, but we don't know this until we explore the *other* local maximas. Blade is an attempt to strike where `wgpu` can't reach, it makes a lot of the opposite design solutions. Try it and see.

## Isn't this going to be slow?

Blade creating a descriptor set (in Vulkan) for each draw call. It doesn't care about pipeline compatibility to preserve the bindings. How is this fast?

Short answer is - yes, it's unlikely going to be faster than wgpu-hal. Long answer is - slow doesn't matter here.

Take a look at Vulkan [performance](performance.md) numbers. wgpu-hal can get 60K bunnies on a slow machine, which is pretty much the maximum. Both wgpu and blade can reach about 20K. Honestly, if you are relying on 20K unique draw calls being fast, you are in a strange place. Generally, developers should switch to instancing or other batching methods whenever the object count grows above 100, not to mention a 1000.

Similar reasoning goes to pipeline switches. If you are relying on many pipeline switches done efficiently, then it's good to reconsider your shaders, perhaps turning into the megashader alley a bit. In D3D12, a pipeline change requires all resources to be rebound anyway (and this is what wgpu-hal/dx12 does regardless of the pipeline compatibility), so this is fine in Blade.
