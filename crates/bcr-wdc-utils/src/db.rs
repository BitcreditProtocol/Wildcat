// ----- standard library imports
// ----- extra library imports
// ----- local imports

// ----- end imports

pub mod surreal {
    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub namespace: String,
        pub database: String,
    }

    /// Renders a timestamp the way stored records serialize it, so bound query
    /// parameters stay comparable against persisted values.
    pub fn tstamp_param(tstamp: crate::TStamp) -> String {
        tstamp
            .format(&time::format_description::well_known::Rfc3339)
            .expect("rfc3339 timestamp")
    }
}

pub mod postgres {
    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct DBConnConfig {
        pub connection: String,
        pub max_connections: u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct StoredRecord {
        #[serde(with = "time::serde::rfc3339")]
        tstamp: crate::TStamp,
    }

    // a bound query parameter must render identically to the stored field, or
    // surreal compares a string against a serialized array and silently matches
    // every row.
    #[test]
    fn tstamp_param_matches_stored_representation() {
        let tstamp = time::macros::datetime!(2026-08-03 12:00:00 UTC);
        let stored = serde_json::to_value(StoredRecord { tstamp }).unwrap();
        assert_eq!(stored["tstamp"], "2026-08-03T12:00:00Z");
        assert_eq!(surreal::tstamp_param(tstamp), "2026-08-03T12:00:00Z");
        assert_eq!(stored["tstamp"], surreal::tstamp_param(tstamp));
    }
}
