# Show images and icons

GPUI provides two elements for graphics:

* [`img`] renders bitmaps (PNG, JPEG, WebP, GIF) and full-color SVGs with their
  original colors intact.
* [`svg`] rasterizes an SVG's alpha channel as a mask filled with the element's
  `text_color`, making monochrome icons follow theme colors automatically.

## Load images from URLs, files, assets, or bytes

[`img`] accepts any value that converts into [`ImageSource`]:

| Argument | Example | Loaded from |
| --- | --- | --- |
| String that parses as a URL | `img("https://example.com/logo.png")` | The app's `HttpClient` |
| Any other string | `img("images/logo.png")` | Exact key in [`AssetRegistry`] |
| `PathBuf` or `&Path` | `img(Path::new("assets/banner.jpg"))` | Local filesystem |
| `Arc<Image>` | `img(Arc::new(Image::from_bytes(..)))` | Encoded bytes in memory, decoded in the background |
| `Arc<RenderImage>` | `img(render_image)` | Pre-decoded BGRA frames, drawn immediately |

Pass a `Path` or `PathBuf` whenever you mean a filesystem path: plain strings
without a URL scheme look up keys in [`AssetRegistry`] rather than opening
files, while Windows drive paths such as `"c:/logo.png"` parse as URL schemes
if passed as strings. Remote URLs require an HTTP client registered via
[`Application::with_http_client`] (installed by default on web targets; native
apps supply their own).

```rust
# use gpui::*;
# use std::sync::Arc;
const DOT: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><circle cx=\"4\" cy=\"4\" r=\"4\" fill=\"#2563eb\"/></svg>";

let remote = img("https://example.com/avatar.png");
let bundled = img("images/logo.png");
let local = img(std::path::PathBuf::from("assets/banner.jpg"));
let inline = img(Arc::new(Image::from_bytes(ImageFormat::Svg, DOT.to_vec()))).size(px(96.));
```

Constructing `img(...)` starts no I/O. Fetching and decoding begin during layout
when GPUI checks the cache; [`Image::from_bytes`] hashes the byte slice so
identical embedded images share one decoded entry. While an image is still
loading, it has zero intrinsic size—give it explicit dimensions so surrounding
layout does not jump when decoding finishes.

## Size, fit, placeholders, and fallbacks

Setting one dimension (`w` or `h`) in pixels or rems scales the other to
preserve the aspect ratio. With a relative dimension such as `w_full()`, the
unset dimension keeps the image's natural size. Set both dimensions to control
the box, and use [`StyledImage::object_fit`] to control how the image occupies it:

* [`ObjectFit::Contain`] *(default)* — scales to fit inside the box without cropping.
* [`ObjectFit::Cover`] — scales to fill the box, cropping excess edges.
* [`ObjectFit::Fill`] — stretches to match both dimensions.
* [`ObjectFit::ScaleDown`] — behaves like `Contain`, never enlarging past natural size.
* [`ObjectFit::None`] — keeps natural pixel dimensions and crops to the box.

Use [`StyledImage::with_loading`] for a loading placeholder and
[`StyledImage::with_fallback`] for missing assets, HTTP errors, or corrupt bytes.
GPUI waits `200ms` ([`LOADING_DELAY`]) before rendering `with_loading` so cached
or fast loads never flash a skeleton. Give the image an ID to retain the loading
state across frames:

```rust
# use gpui::*;
let photo = img("https://example.com/photo.jpg")
    .id("photo")
    .size(px(240.))
    .object_fit(ObjectFit::Cover)
    .with_loading(|| div().size(px(240.)).bg(rgb(0xe5e7eb)).into_any_element())
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

## Tintable icons with `svg`

[`svg`] masks the element's `text_color` through the SVG's alpha channel. Load
the SVG from [`AssetRegistry`] with [`Svg::path`], from disk in the background
with [`Svg::external_path`], or from static bytes with [`Svg::data`]:

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

[`Svg::with_transformation`] scales, rotates, or translates the rendered vector
without altering its layout box.

## Register bundled assets

Build an [`AssetRegistry`] and pass it to [`Application::with_assets`]. Entries
can hold borrowed static slices (`include_bytes!`), owned byte buffers, or
`Send + Sync` closures invoked on demand:

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

To embed an entire directory at compile time, enable the `embedded-assets` Cargo
feature on `gpui-ce`, derive `rust_embed::RustEmbed` on a folder struct, and
pass `AssetRegistry::iter_embed::<Assets>()` to [`AssetRegistry::extend`].

```rust,no_run
# use gpui::*;
# fn assets() -> AssetRegistry { AssetRegistry::default() }
fn main() {
    gpui_platform::application()
        .with_assets(assets())
        .run(|_cx: &mut App| {});
}
```

## Caching and animated images

Resource loads—including failures—stay in the app-wide cache until evicted. If a
broken file or remote endpoint recovers, call [`ImageSource::remove_asset`]
before re-rendering to retry.

When a gallery or transient view loads many URLs or files whose decoded textures
should drop when the view closes, wrap the subtree in [`image_cache`] with
[`retain_all`]:

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

`retain_all` ties resource cache entries to that element ID's state; unmounting
the gallery or changing its ID releases its cached references (encoded
`Arc<Image>` sources still use the app-wide decode cache). Custom [`ImageCache`]
implementations can also attach to individual elements via [`Img::image_cache`].

Give an animated image an ID to retain its current frame and playback time.
Animated GIF and WebP resources loaded from URLs, paths, or [`AssetRegistry`]
then loop while the window is active and [`App::reduce_motion`] is
`false`. Animated GIFs passed via `Image::from_bytes` also animate; animated
WebP passed via `Image::from_bytes` decodes only the first frame.

## See also

* [Images example](crate::_documentation::examples::images) — object fit modes,
  byte sources, and tinted icons (`cargo run --example images`).
* [Custom drawing](crate::_documentation::guides::custom_drawing) — painting a
  [`RenderImage`] manually inside a `canvas`.
* [Accessibility](crate::_documentation::guides::accessibility) — labeling
  meaningful images and icons for assistive technologies.
