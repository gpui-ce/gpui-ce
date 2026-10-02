#![cfg(target_os = "windows")]

mod clipboard;
mod destination_list;
mod direct_manipulation;
mod directx_atlas;
mod directx_devices;
// Kept compiled under the `wgpu` feature too: `direct_write` shares its
// shader/emoji-rasterization internals. When the `wgpu` feature selects the
// WGPU renderer, `DirectXRenderer` is simply never instantiated.
#[cfg_attr(feature = "wgpu", allow(dead_code))]
mod directx_renderer;
mod dispatcher;
mod display;
mod events;
mod font_rasterizer;
mod keyboard;
mod platform;
mod system_notifications;
mod system_settings;
mod util;
mod vsync;
#[cfg(feature = "wgpu")]
mod wgpu_renderer;
mod window;
mod wrapper;

pub(crate) use clipboard::*;
pub(crate) use destination_list::*;
pub(crate) use directx_atlas::*;
pub(crate) use directx_devices::*;
pub(crate) use directx_renderer::*;
pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use events::*;
pub(crate) use font_rasterizer::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use system_notifications::*;
pub(crate) use system_settings::*;
pub(crate) use util::*;
pub(crate) use vsync::*;
pub(crate) use window::*;
pub(crate) use wrapper::*;

#[cfg(feature = "wgpu")]
pub(crate) use wgpu_renderer::WindowsWgpuRenderer as WindowRenderer;
#[cfg(feature = "wgpu")]
pub(crate) use wgpu_renderer::Context as RendererContext;
#[cfg(not(feature = "wgpu"))]
pub(crate) use directx_renderer::DirectXRenderer as WindowRenderer;

pub use platform::WindowsPlatform;

pub(crate) use windows::Win32::Foundation::HWND;
