use early_panic::early_panic;
use std::str::FromStr;

#[early_panic]
fn test_one_payload() -> Result<u64, <u64 as FromStr>::Err> {
    let v = "fail42".parse::<u64>()?;
    Ok(v)
}

#[test]
#[should_panic]
fn test_one() {
    let _ = test_one_payload();
}
