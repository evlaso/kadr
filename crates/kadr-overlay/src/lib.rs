//! The area-selection overlay (SPEC §3.8). So far its model: the
//! selection and what the pointer and keys do to it, without a window or a
//! UI framework, which the app's GPUI picker draws. Its own windows
//! (layer-shell on Wayland, winit elsewhere) and CPU drawing come next.

pub mod model;
