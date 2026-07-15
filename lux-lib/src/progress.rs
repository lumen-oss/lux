use std::{borrow::Cow, sync::Arc, time::Duration};

use crate::{config::Config, logging::LoggingState};

mod private {
    pub trait HasProgress {}
}

impl private::HasProgress for () {}
impl private::HasProgress for MultiProgress {}
impl private::HasProgress for ProgressBar {}

pub trait HasProgress: private::HasProgress {}
impl<P: private::HasProgress> HasProgress for P {}

#[derive(Clone)]
pub struct Progress<T: HasProgress>(ProgressImpl<T>);

#[derive(Clone)]
enum ProgressImpl<T: HasProgress> {
    NoProgress,
    Progress(T),
}

impl<T> Progress<T>
where
    T: HasProgress,
{
    pub fn new(progress: T, config: &Config) -> Progress<T> {
        Progress(if config.no_progress() {
            ProgressImpl::NoProgress
        } else {
            ProgressImpl::Progress(progress)
        })
    }

    pub fn no_progress() -> Progress<T> {
        Progress(ProgressImpl::NoProgress)
    }

    pub fn map<F, R>(&self, callback: F) -> Progress<R>
    where
        F: FnOnce(&T) -> R,
        R: HasProgress,
    {
        Progress(if let ProgressImpl::Progress(progress) = &self.0 {
            ProgressImpl::Progress(callback(progress))
        } else {
            ProgressImpl::NoProgress
        })
    }
}

// WARNING: Don't implement `Clone` for this.
pub struct MultiProgress(indicatif::MultiProgress);
pub struct ProgressBar(indicatif::ProgressBar);

impl MultiProgress {
    pub fn new(config: &Config) -> Progress<Self> {
        Progress::new(Self(indicatif::MultiProgress::new()), config)
    }

    pub fn new_arc(config: &Config) -> Arc<Progress<MultiProgress>> {
        Arc::new(MultiProgress::new(config))
    }

    pub fn add(&self, bar: ProgressBar) -> ProgressBar {
        ProgressBar(self.0.insert_from_back(0, bar.0))
    }

    pub fn new_bar(&self) -> ProgressBar {
        self.add(ProgressBar::new())
    }

    pub fn suspend<F, R>(&self, callback: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.0.suspend(callback)
    }
}

impl ProgressBar {
    pub fn new() -> Self {
        let bar =
            indicatif::ProgressBar::new_spinner().with_finish(indicatif::ProgressFinish::AndClear);

        if let LoggingState::Disabled = crate::logging::state() {
            // NOTE: this spawns a background thread which causes rendering (we don't want that)
            bar.enable_steady_tick(Duration::from_millis(100));
        }

        Self(bar)
    }

    pub fn into_raw(self) -> indicatif::ProgressBar {
        self.0
    }

    pub fn set_message<M>(&self, message: M)
    where
        M: Into<Cow<'static, str>>,
    {
        let msg: Cow<'static, str> = message.into();
        match crate::logging::state() {
            crate::logging::LoggingState::Enabled => {
                crate::logging::push(
                    crate::logging::LogLevel::Info,
                    msg.clone().into_owned(),
                    Some("progress".to_string()),
                );
            }
            crate::logging::LoggingState::Disabled => {
                self.0.set_message(msg);
            }
        }
    }

    pub fn set_position(&self, position: u64) {
        self.0.set_position(position)
    }

    pub fn position(&self) -> u64 {
        self.0.position()
    }

    pub fn println<M>(&self, message: M)
    where
        M: AsRef<str>,
    {
        match crate::logging::state() {
            crate::logging::LoggingState::Enabled => {
                crate::logging::push(
                    crate::logging::LogLevel::Info,
                    message.as_ref().to_string(),
                    Some("progress".to_string()),
                );
            }
            crate::logging::LoggingState::Disabled => {
                self.0.println(message.as_ref());
            }
        }
    }

    pub fn finish_with_message<M>(&self, message: M)
    where
        M: Into<Cow<'static, str>>,
    {
        let msg: Cow<'static, str> = message.into();
        match crate::logging::state() {
            crate::logging::LoggingState::Enabled => {
                crate::logging::push(
                    crate::logging::LogLevel::Info,
                    msg.clone().into_owned(),
                    Some("progress".to_string()),
                );
            }
            crate::logging::LoggingState::Disabled => {
                self.0.finish_with_message(msg);
            }
        }
    }

    pub fn finish_and_clear(&self) {
        self.0.finish_and_clear()
    }
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self::new()
    }
}

impl From<String> for ProgressBar {
    fn from(message: String) -> Self {
        Self(Self::new().0.with_message(message))
    }
}

impl From<u64> for ProgressBar {
    fn from(position: u64) -> Self {
        let new = Self::new();
        new.set_position(position);

        new
    }
}
