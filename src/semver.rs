use semver::Version;

pub struct Semver {
    version: semver::Version,
}

impl Semver {
    pub fn new(version: semver::Version) -> Self {
        Self { version }
    }
}
