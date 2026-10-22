//! Recordings and the studio with no UI: a recording's log (`track`), the
//! camera's look (`camera`), the studio's edit and how it becomes an ffmpeg
//! graph (`studio`), and running ffmpeg with progress (`job`). What the
//! machine can do (an NVIDIA encoder) is the caller's to say.

#[macro_use]
extern crate kadr_core;

pub mod camera;
pub mod job;
pub mod studio;
pub mod track;
