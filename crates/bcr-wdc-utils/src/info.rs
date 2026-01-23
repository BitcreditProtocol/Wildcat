// ----- standard library imports
use std::{collections::HashMap, str::FromStr};
// ----- extra library imports
// ----- local imports
use crate::built_info;

// ----- end imports

pub fn get_build_time() -> chrono::DateTime<chrono::Utc> {
    built::util::strptime(built_info::BUILT_TIME_UTC)
}

const EBILL_CORE: &str = "bcr-ebill-core";
pub fn get_ebill_version() -> Option<built::semver::Version> {
    let version = built::util::parse_versions(&built_info::DEPENDENCIES).find_map(|(n, v)| {
        if n == EBILL_CORE {
            Some(v)
        } else {
            None
        }
    });
    version
}

pub fn get_deps_versions() -> HashMap<String, Option<built::semver::Version>> {
    HashMap::from([(String::from(EBILL_CORE), get_ebill_version())])
}

pub fn get_version() -> built::semver::Version {
    let version = built_info::PKG_VERSION;
    let default = built::semver::Version::new(0, 0, 0);
    built::semver::Version::from_str(version).unwrap_or(default)
}
