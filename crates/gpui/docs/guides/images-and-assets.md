# Show images and icons

[`img`] preserves an image's colors, including those in an SVG. [`svg`] makes
an icon follow the theme: it takes the SVG's alpha channel as a mask and fills
it with the element's text color. Choose the element for that treatment, then
choose a source according to where the bytes live.

## Choose a source

[`img`] accepts anything that converts to an [`ImageSource`]. The source type determines
where GPUI obtains bytes and therefore which external dependency can fail:

| Source | Example | Loaded from |
| --- | --- | --- |
| A string that parses as a URL | `img("https://example.com/logo.png")` | The HTTP client |
| Any other string | `img("images/logo.png")` | The [`AssetRegistry`] of the app |
| A `PathBuf` or `&Path` | `img(path)` | The file system, relative to the working directory |
| An [`Image`] in an `Arc` | `img(Arc::new(Image::from_bytes(..)))` | Bytes you already have |
| A [`RenderImage`] in an `Arc` | `img(render_image)` | Nothing, it is decoded already |

For example, `img("images/logo.png")` asks the registry for that exact key.
It does not open a file. Pass a `PathBuf` or `&Path` for a filesystem path;
this also avoids a string being accepted as a URL when you intended a path.

An [`Image`] owns encoded bytes for background decoding. A [`RenderImage`]
already contains pixels and dimensions. Neither fetches a resource. A URL,
asset or file source must obtain its bytes from the named service, while a
warm cache can make a previously loaded or decoded image available at once.

URLs use the app's HTTP client, configured with `Application::with_http_client`. If no
client is installed, the request fails and the error fallback is shown. The web
`gpui_platform` constructors install one; native applications must provide one before
loading URLs.

```rust
# use gpui::*;
let remote = img("https://example.com/avatar.png");
let bundled = img("images/logo.png");
let local = img(std::path::PathBuf::from("assets/banner.jpg"));
```

Both resource loading and encoded-image decoding begin during layout, when
GPUI checks the appropriate cache; constructing the element starts neither.
A completed entry can draw, and a missing entry starts the background work. Completion notifies the
view for a later frame. On single-threaded web builds that background work
still runs on the browser's main thread, as [executors](crate::_documentation::concepts::executors)
explains. Until the image arrives, it has no intrinsic size. Reserve space
with dimensions or a placeholder to avoid moving the surrounding layout.

## Embed image bytes

Use an [`Image`] when the bytes already belong to your program, such as data from
`include_bytes!`, and specify the [`ImageFormat`] that should decode them. This inline SVG
is still a full-color image because it goes through `img`:

```rust
# use gpui::*;
# use std::sync::Arc;
const DOT: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><circle cx=\"4\" cy=\"4\" r=\"4\" fill=\"#2563eb\"/></svg>";

let dot = Arc::new(Image::from_bytes(ImageFormat::Svg, DOT.to_vec()));
let icon = img(dot.clone()).size(px(96.));
```

[`Image::from_bytes`] takes ownership of the bytes and hashes them for an ID, so identical
byte sequences can share one decoded copy. [`ImageFormat::Svg`] keeps the SVG on the
full-color image path; choose [`svg`] when it should instead become a tintable mask.

## Give the image a stable box

Once decoded, the natural dimensions determine the image's size unless you
supply dimensions. One explicit dimension preserves its aspect ratio. Two
explicit dimensions define a box, with [`StyledImage::object_fit`] choosing
how the image occupies it. The [`ObjectFit`] choices are:

- [`ObjectFit::Contain`] (the default) scales to fit inside, with empty space.
- [`ObjectFit::Cover`] scales to fill, cropping the overflow.
- [`ObjectFit::Fill`] stretches to the box and ignores the aspect ratio.
- [`ObjectFit::ScaleDown`] is `Contain`, but never enlarges past natural size.
- [`ObjectFit::None`] keeps natural size and crops to the box.

```rust
# use gpui::*;
let avatar = img("images/avatar.png")
    .size(px(48.))
    .rounded_full()
    .object_fit(ObjectFit::Cover);

let disabled_logo = img("images/logo.png").size(px(120.)).grayscale(true);
```

## Show a placeholder while loading, and a fallback on error

Once the box is stable, choose what it contains while a resource is unavailable.
[`StyledImage::with_loading`] supplies a placeholder, but GPUI waits 200 milliseconds
(`LOADING_DELAY`) before showing it, so a quick request does not flash. If loading fails,
[`StyledImage::with_fallback`] supplies the replacement tree. Missing assets, non-success
HTTP statuses, and undecodable bytes all use that path. The
[`HttpClient`](crate::http_client::HttpClient) returns the status and body; the image loader
checks the status before attempting to decode:

```rust
# use gpui::*;
let photo = img("https://example.com/photo.jpg")
    .size(px(240.))
    .with_loading(|| {
        div()
            .size(px(240.))
            .bg(rgb(0xe5e7eb))
            .into_any_element()
    })
    .with_fallback(|| {
        div()
            .size(px(240.))
            .flex()
            .items_center()
            .justify_center()
            .child("Image unavailable")
            .into_any_element()
    });
```

Both closures return an [`AnyElement`]. Layout may call them more than once,
so keep construction cheap. The loading tree occupies the image's place only
after the delay; a failed request uses the fallback tree. Reserving the same
240px box in all three states keeps this photo from moving its neighbors.

## Draw icons with `svg`

[`svg`] paints the alpha channel of an SVG as a mask, filled with the element's
text color. That makes it the right tool for icons that follow the theme. It
takes its file from one of three places:

- [`Svg::path`] names an asset in the [`AssetRegistry`], as `img` does with
  asset paths.
- [`Svg::external_path`] reads a file from the file system.
- [`Svg::data`] takes the bytes directly, which suits an icon you include with
  `include_bytes!`.

```rust
# use gpui::*;
const STAR: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><rect width=\"8\" height=\"8\"/></svg>";

let search_icon = svg()
    .path("icons/search.svg")
    .size(px(16.))
    .text_color(rgb(0x2563eb));

let star = svg()
    .data(STAR)
    .size(px(16.))
    .text_color(rgb(0xf59e0b));
```

Choose between `img` and `svg` according to whether the original colors should survive.
All three `svg` sources produce the same tintable mask; they differ in when bytes become
available. [`Svg::data`] already has the bytes, [`Svg::path`] reads the registry on first
draw and caches the result, and [`Svg::external_path`] reads the file in the background.
[`Svg::with_transformation`] rotates, scales, or moves the rendering without changing
layout.

## Register assets with the app

Give the app an [`AssetRegistry`] with [`Application::with_assets`]. An asset path is an
exact lookup key—any string is allowed, such as `icons/star.svg`—rather than a path
resolved against the working directory. Build the registry with [`AssetRegistry::insert`]; an entry can hold
static bytes, bytes loaded at startup, or a loader to call on demand:

```rust
# use gpui::*;
# use std::borrow::Cow;
let mut assets = AssetRegistry::default();
assets
    .insert(
        "icons/star.svg",
        Cow::Borrowed(
            &b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><rect width=\"8\" height=\"8\"/></svg>"[..],
        ),
    )
    .unwrap();

assert!(assets.load("icons/star.svg").is_some());
assert!(assets.load("icons/missing.svg").is_none());
```

`insert` returns an error for a duplicate key. An on-demand loader runs on
each registry load, but an `img` may reuse its own cached result without
loading from the registry again. Keep those two lifetimes separate when
implementing asset reloads.

Pass the completed registry to the application builder:

```rust,no_run
# use gpui::*;
# fn assets() -> AssetRegistry { AssetRegistry::default() }
fn main() {
    gpui_platform::application()
        .with_assets(assets())
        .run(|_cx: &mut App| {
            // Open windows here. Images and icons load the assets above.
        });
}
```

To bundle a whole folder into the binary, enable the `embedded-assets` feature of
`gpui` and add `rust-embed` to your own crate. Derive `RustEmbed` for a type that
points at the folder, then load it with `AssetRegistry::iter_embed` and
[`AssetRegistry::extend`]. Paths are the file names relative to that folder.

## Cache images

A second `img` for the same URL, path or asset reuses the first request's
result. That includes an error. Repairing a file or restoring a server does
not by itself replace a cached failure, so the fallback can remain visible
after the cause is fixed. For the app-wide cache, evict that source with
[`ImageSource::remove_asset`] before requesting it again.

For a gallery whose resource cache should live with its UI, wrap the images in
[`image_cache`] with a [`retain_all`] provider. The provider keeps a cache in window
element state: the same ID in the same element-ID scope reuses it across frames; a new
ID creates another cache. When that state disappears, the cache releases its image
references. Images retained by another owner can outlive the gallery.

A scoped failure belongs to that gallery's cache. Evicting the app-wide
entry with [`ImageSource::remove_asset`] leaves it intact; drop the gallery's
subtree or change the [`retain_all`] ID to retry there. Only resource sources
use this scoped cache. Encoded [`Image`] bytes go through the app-wide
decoded-image cache even inside the gallery.

```rust
# use gpui::*;
let gallery = image_cache(retain_all("gallery")).children(
    (0..3).map(|ix| {
        img(format!("https://example.com/photos/{ix}.jpg"))
            .size(px(120.))
            .into_any_element()
    }),
);
```

If you write your own [`ImageCache`], attach it to one image with
[`Img::image_cache`].

## Animated images

GIF and animated WebP resources decode into frames, and [`img`] plays them in a loop when
they come from a URL, path, or asset name. An animated GIF passed as [`Image`] bytes also
keeps its frames, but an animated WebP passed as [`Image`] bytes uses static decoding and
shows only its first frame. While the window is active and [`App::reduce_motion`] is
false, the element requests the next frame itself. Still images, and animated formats
that use static decoding, show their first frame.

## See also

- [Images example](crate::_documentation::examples::images), which shows object
  fit, byte sources and tinted icons. Run it with `cargo run --example images`.
- [Text](mod@crate::_documentation::guides::text) for placing icons next to labels.
- [Custom drawing](crate::_documentation::guides::custom_drawing) for drawing an
  image inside a `canvas` with [`Window::paint_image`].
- [Accessibility](crate::_documentation::guides::accessibility) for alternative
  text on images that carry meaning.
