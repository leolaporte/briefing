use anyhow::{anyhow, Result};
use chrono::{Datelike, Local, TimeZone, Timelike, Utc};

/// Convert the current local wall-clock timestamp to a UTC datetime with the
/// same date/time fields. This preserves "local schedule semantics" when
/// downstream logic expects UTC input.
pub fn local_wallclock_as_utc() -> Result<chrono::DateTime<Utc>> {
    let local_now = Local::now();
    Utc.with_ymd_and_hms(
        local_now.year(),
        local_now.month(),
        local_now.day(),
        local_now.hour(),
        local_now.minute(),
        local_now.second(),
    )
    .single()
    .ok_or_else(|| anyhow!("Unable to construct UTC timestamp from local wall-clock time"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_wallclock_conversion_preserves_components() {
        let converted = local_wallclock_as_utc().unwrap();
        let local_now = Local::now();
        assert_eq!(converted.year(), local_now.year());
        assert_eq!(converted.month(), local_now.month());
        assert_eq!(converted.day(), local_now.day());
    }
}
