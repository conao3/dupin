//! The macOS side of Dupin: the providers that need the system (installed apps, Spotlight)
//! and the actions that hand results to it (NSWorkspace).

pub mod apps;
pub mod spotlight;
pub mod workspace;

pub use apps::AppsProvider;
pub use spotlight::{SpotlightProvider, Target};
