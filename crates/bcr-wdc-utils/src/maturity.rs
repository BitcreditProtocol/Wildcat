// ----- standard library imports
// ----- extra library imports
use bcr_common::TStamp;
// ----- local imports

// ----- end imports

pub fn credit_expires_at(maturity_date: time::Date) -> TStamp {
    maturity_date.midnight().assume_utc()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_expires_when_the_bill_matures() {
        let maturity = time::macros::date!(2026 - 09 - 13);
        let expires = credit_expires_at(maturity);
        assert_eq!(expires.date(), maturity);
        assert_eq!(expires.time(), time::Time::MIDNIGHT);
        assert!(expires < credit_expires_at(maturity.next_day().unwrap()));
    }
}
