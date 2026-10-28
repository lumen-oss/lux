pub mod c_files;
pub mod c_link;
pub mod c_modules;

use std::path::PathBuf;

use crate::config::Config;

pub(crate) trait CCExt {
    fn try_compile_objects(&self, config: &Config) -> Result<Vec<PathBuf>, cc::Error>;
}

impl CCExt for cc::Build {
    fn try_compile_objects(&self, config: &Config) -> Result<Vec<PathBuf>, cc::Error> {
        if config.verbose() {
            self.try_compile_intermediates()
        } else {
            // HACK: try_compile_intermediates prints the exit code to stderr,
            // which interferes with lux-cli progress spinners, etc
            let mut _stdout_gag = gag::Gag::stdout();
            let mut _stderr_gag = gag::Gag::stderr();
            self.try_compile_intermediates()
        }
    }
}
