# Motivation

## Goal

Have a layer for graphics programming for those who know what they are doing, and who wants to get the stuff working fast. It's highly opinionated and ergonomic, but also designed specifically for mid to high range hardware and modern APIs. Today, the alternatives are either too high level (engines), too verbose (APIs directly), or just overly general.

Opinionated means the programming model is very limited. But if something is written against this model, we want to guarantee that it's going to run very efficient, more efficient than any of the more general alternatives would do.

This is basically a near-perfect graphics layer for myself, which I'd be happy to use on my projects. I hope it can be useful to others, too.

## Alternatives

*wgpu* provides the most thorough graphics abstraction in Rust ecosystem. The main API is portable over pretty much all the (open) platforms, including the Web. However, it is very restricted (by being a least common denominator of the platforms), fairly verbose (possible to write against it directly, but not quite convenient), and has overhead (for safety and portability).

*wgpu-hal* provides an unsafe portable layer, which has virtually no overhead. The point about verbosity still applies. It's possible to write a more ergonomic layer on top of wgpu-hal, but one can't cut the corners embedded in wgpu-hal's design. For example, wgpu-hal expects resource states to be tracked by the user and changed (on a command encoder) explicitly.

*rafx* attempts to offer a good vertically integrated engine with multiple backends. *rafx* itself is too high level, while *rafx-api* is too low level and verbose.

*sierra* abstracts over Vulkan. It has great ergonomic features (some expressed via procedural macros). Essentially it has the same problem (for the purpose of fitting our goal) - choice is between low level overly generic API and a high-level one (*arcana*).

Finally, we don't consider GL-based abstractions, such as *luminance*, since the API is largely outdated.

# Design

The API is supposed to be minimal, targeting the capabilities of mid to high range machines on popular platforms. It's also totally unsafe, assuming the developer knows what they are doing. We realy on native API validation to assist developers.

## Compromises

*Object lifetime* is explicit, no automatic tracking is done. This is similar to most of the alternatives.

*Object memory* is automatically allocated based on a few profiles.

Basic *resources*, such buffers and textures, are small `Copy` structs.

*Resource states* do not exist. The API is built on an assumption that the driver knows better how to track resource states, and so our API doesn't need to care about this. The only command exposed is a catch-all barrier.

*Bindings* are pushed directly to command encoders. This is similar to Metal Argument Buffers. There are no descriptor sets or pools. You take a structure and push it to the state. This structure includes any uniform data directly. Changing a pipeline invalidates all bindings, just like in DX12.

In addition, several features may be added late or not added at all for the sake of keeping everything simple:

  - vertex buffers (use storage buffers instead)
  - multisampling (too expensive)

## Backends

At first, the API should run on Vulkan and Metal. There is no DX12 support planned.

On Metal side we want to take advantage of the argument buffers if available.

On Vulkan we'll require certain features to make the translation simple:

  - [VK_KHR_push_descriptor](https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_KHR_push_descriptor.html)
  - [VK_KHR_descriptor_update_template](https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_KHR_descriptor_update_template.html)
  - [VK_EXT_inline_uniform_block](https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_EXT_inline_uniform_block.html)
  - [VK_KHR_dynamic_rendering](https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_KHR_dynamic_rendering.html)

## Assumptions

Blade is based on different set of assumptions from wgpu-hal:
- *safety*: wgpu places safety first and foremost. Self-sufficient, guarantees no UB. Blade is on the opposite - considers safety to be secondary. Expects users to rely on native API's validation and tooling.
- *API reach*: wgpu attempts to be everywhere, having backends for all the APIs it can reach. Blade targets only the essential backends: Vulkan and Metal.
- *abstraction*: wgpu is completely opaque, with only a few unsafe APIs for interacting with external objects. Blade needs to be transparent, since it assumes modifcation by the user, and doesn't provide safety.
- *errors*: wgpu considers all external errors recoverable. Blade doesn't expect any recovery after the initialization is done.
- *object copy*: wgpu-hal hides API objects so that they can only be `Clone`, and some of the backends use `Arc` and other heap-allocated backing for them. Blade keeps the API for resources to be are light as possible and allows them to be copied freely.
- *bind group creation cost*: wgpu considers it expensive, needs to be prepared ahead of time. Blade considers it cheap enough to always create on the fly.
| bind group invalidation | should be avoided by following pipeline compatibility rules | everything is re-bound on pipeline change |
- *barriers*: wgpu attempts to always use the optimal image layouts and can set reduced access flags on resources based on use. Placing the barriers optimally is a non-trivial task to solve, no universal solutions. Blade not only ignores this fight by making the user place the barrier, these barriers are only global, and there are no image layout changes - everything is GENERAL.
- *usage*: wgpu expects to be used as a Rust library. Blade expects to be vendored in and modified according to the needs of a user. Hopefully, some of the changes would appear upstream as PRs.

In other words, this is a bit **experiment**. It may fail horribly, or it may open up new ideas and perspectives.
