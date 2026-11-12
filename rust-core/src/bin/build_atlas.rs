//! `build-atlas` — generates the offline encyclopedia shipped in the APK.
//!
//! ```text
//! cargo run --release --bin build-atlas [-- <output.db>]
//! ```
//! Default output: `<repo>/app/src/main/assets/atlas.db`.
//!
//! The database is written to `<output>.tmp` with journaling and fsync disabled, then
//! compacted (`VACUUM`) and analysed (`ANALYZE`) and finally renamed over the target,
//! so an interrupted run never leaves a half-written asset behind.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use atlas_core::dataset::{self, SeedStats};
use atlas_core::db::{self, Database};
use atlas_core::{geo, HabitatType};
use rusqlite::Connection;

/// Shipped asset is built once, so favour size over compression speed.
const ASSET_ZSTD_LEVEL: i32 = 19;

fn default_output() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("app")
        .join("src")
        .join("main")
        .join("assets")
        .join("atlas.db")
}

fn build(out: &Path) -> Result<SeedStats, Box<dyn Error>> {
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if tmp.exists() {
        // Leftover of an interrupted run of this tool.
        fs::remove_file(&tmp)?;
    }

    let stats = {
        let conn = Connection::open(&tmp)?;
        let mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "OFF", |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("off") {
            return Err(format!("journal_mode is {mode}, expected off").into());
        }
        conn.pragma_update(None, "synchronous", "OFF")?;

        db::init_schema(&conn)?;
        let tx = conn.unchecked_transaction()?;
        let stats = dataset::seed_catalog_with(&tx, ASSET_ZSTD_LEVEL)?;
        tx.commit()?;

        // Merge FTS5 segments into one b-tree for the read-only asset.
        conn.execute_batch("INSERT INTO taxa_fts(taxa_fts) VALUES('optimize');")?;
        conn.execute_batch("VACUUM; ANALYZE;")?;
        stats
    };

    fs::rename(&tmp, out)?;
    Ok(stats)
}

/// Re-opens the generated file through the production code path and runs smoke queries.
fn verify(out: &Path) -> Result<(), Box<dyn Error>> {
    let path = out.to_str().ok_or("output path is not valid UTF-8")?;
    let atlas = Database::new_from_file(path)?;

    let marine = atlas.list_taxa_by_habitat(HabitatType::Marine, 10)?;
    if marine.is_empty() {
        return Err("no marine taxa in the generated atlas".into());
    }
    let spb = geo::lat_lon_to_h3_res4(59.93, 30.33).ok_or("invalid probe coordinates")?;
    if atlas.lookup_ecoregions(spb)?.is_none() {
        return Err("h3_geography lacks the Saint Petersburg anchor".into());
    }
    if atlas.search_taxa("цикута", 5)?.is_empty() {
        return Err("FTS5 index does not find 'цикута'".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_output);

    let result = build(&out).and_then(|stats| verify(&out).map(|()| stats));
    match result {
        Ok(stats) => {
            let size = fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!("atlas.db written: {}", out.display());
            println!(
                "  taxa: {}, exact presence cells: {}, region bindings: {}, h3_geography cells: {}",
                stats.taxa, stats.presence_cells, stats.region_bindings, stats.geography_cells
            );
            println!("  size: {:.1} KiB", size as f64 / 1024.0);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("build-atlas failed: {e}");
            ExitCode::FAILURE
        }
    }
}
