use std::fs::{read_dir, remove_file, File, OpenOptions, ReadDir};

use csv::Writer;

use crate::utils::tester::run_test;

#[tokio::test]
async fn benchmark_case_1() -> anyhow::Result<()> {
    // Paths are relative to the Cargo.toml
    // Smallest real data test case
    run_test(
        "./tests/data/283221487/initial.json".to_string(),
        "./tests/data/283221487/transactions.json".to_string(),
    )
    .await;

    Ok(())
}

// Default ignore since it is slow to run all the tests and save to file.
#[tokio::test]
#[ignore]
async fn benchmark_all() -> anyhow::Result<()> {
    // Clear existing if it exists.
    let _ = remove_file("benchmark_results.csv");
    let file: File = OpenOptions::new()
        .write(true)
        .create(true)
        .open("benchmark_results.csv")
        .expect("Create new benchmark file");

    let mut writer: Writer<File> = Writer::from_writer(file);
    writer
        .write_record(&[
            "PHX".to_string(),
            "MFX".to_string(),
            "num_orders".to_string(),
            "signature".to_string(),
        ])
        .expect("Write headers");
    writer.flush().expect("Flush");

    let paths: ReadDir = read_dir("./tests/data").expect("List dir for testdata");
    for path in paths {
        let slot_dir: String = path.expect("Get path").path().display().to_string();

        println!("Testing {}", slot_dir);
        run_test(
            format!("{}/initial.json", slot_dir),
            format!("{}/transactions.json", slot_dir),
        )
        .await;
    }

    Ok(())
}
