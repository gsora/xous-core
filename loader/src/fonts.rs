#![cfg_attr(rustfmt, rustfmt_skip)]
// DO NOT MAKE EDITS HERE because this file is automatically generated.
// The order of these modules affects the link order in the loader, which is referred to in the graphics engine.
// To make changes, see <xous_root>/services/graphics-server/src/blitstr2/codegen/main.go

#[cfg(not(feature = "cramium-soc"))]
pub mod emoji;
#[cfg(not(feature = "cramium-soc"))]
pub mod zh;
#[cfg(not(feature = "cramium-soc"))]
pub mod ja;
#[cfg(not(feature = "cramium-soc"))]
pub mod kr;
#[cfg(not(feature = "cramium-soc"))]
pub mod bold;
#[cfg(not(feature = "cramium-soc"))]
pub mod mono;
#[cfg(not(feature = "cramium-soc"))]
pub mod regular;
#[cfg(not(feature = "cramium-soc"))]
pub mod tall;
#[cfg(not(feature = "cramium-soc"))]
pub mod small;

#[cfg(feature = "cramium-soc")]
pub mod emoji;
#[cfg(feature = "cramium-soc")]
pub mod bold;
#[cfg(feature = "cramium-soc")]
pub mod mono;
#[cfg(feature = "cramium-soc")]
pub mod regular;
#[cfg(feature = "cramium-soc")]
pub mod tall;
#[cfg(feature = "cramium-soc")]
pub mod small;
