//! A server file wherever the app shows one (a chip, a picture, a task's result, the viewer, the
//! report panel): its uses (Share…, Save to Downloads, Save As…, Open in its app, Copy) as
//! buttons or a right-click menu, dragging it out to another app as the real file, and the
//! status line saying how its download and its use went.

pub(crate) mod actions;
mod default_app;
pub(crate) mod drag;
pub(crate) mod status_line;
