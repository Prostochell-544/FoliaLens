use std::sync::{Mutex, MutexGuard};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    dataset, geo, AtlasError, HabitatType, Kingdom, SafetyStatus, TaxonDetail, TaxonSummary,
};

/// Number of highest raw logits that reach the SQL spatial filter. Planetary models emit
/// 10k–50k logits; only these candidates are looked up, so the cost of
/// [`Database::postprocess_vision_logits`] is bounded regardless of model size.
pub const VISION_PREFILTER_TOP_N: usize = 32;

/// Probability multiplier for candidates outside their range or season.
const GEO_PENALTY: f32 = 0.15;

/// Biogeographic regionalisation stored in `taxon_regions.region_scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegionScheme {
    /// TDWG / WGSRPD Level 3 botanical countries.
    Tdwg,
    /// WWF Terrestrial Ecoregions of the World.
    Teow,
    /// Freshwater Ecoregions of the World.
    Feow,
    /// Marine Ecoregions of the World.
    Meow,
}

impl RegionScheme {
    pub const ALL: [RegionScheme; 4] = [
        RegionScheme::Tdwg,
        RegionScheme::Teow,
        RegionScheme::Feow,
        RegionScheme::Meow,
    ];

    pub const fn code(self) -> i64 {
        match self {
            RegionScheme::Tdwg => 0,
            RegionScheme::Teow => 1,
            RegionScheme::Feow => 2,
            RegionScheme::Meow => 3,
        }
    }

    /// Largest valid region id in this scheme (ids are `1..=max_id`; `0` = not applicable).
    pub const fn max_id(self) -> i64 {
        match self {
            RegionScheme::Tdwg => 369,
            RegionScheme::Teow => 867,
            RegionScheme::Feow => 426,
            RegionScheme::Meow => 232,
        }
    }
}

/// One `h3_geography` row: the regions an H3 cell belongs to (`0` = not applicable,
/// e.g. no MEOW zone for an inland cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellEcoregions {
    pub tdwg_code: i64,
    pub teow_biome: i64,
    pub feow_basin: i64,
    pub meow_zone: i64,
}

impl CellEcoregions {
    /// Region id of the cell in `scheme`, or `None` when not applicable.
    pub fn region(&self, scheme: RegionScheme) -> Option<i64> {
        let id = match scheme {
            RegionScheme::Tdwg => self.tdwg_code,
            RegionScheme::Teow => self.teow_biome,
            RegionScheme::Feow => self.feow_basin,
            RegionScheme::Meow => self.meow_zone,
        };
        (id != 0).then_some(id)
    }
}

fn db_err(e: rusqlite::Error) -> AtlasError {
    AtlasError::DbError(e.to_string())
}

/// Returns up to `n` `(class_index, logit)` pairs with the highest logits, unordered.
///
/// O(len) partial selection via `select_nth_unstable_by`; NaN logits rank lowest.
pub fn top_n_logits(logits: &[f32], n: usize) -> Vec<(u32, f32)> {
    if n == 0 {
        return Vec::new();
    }
    let mut pairs: Vec<(u32, f32)> = logits
        .iter()
        .take(u32::MAX as usize)
        .enumerate()
        .map(|(i, &l)| (i as u32, if l.is_nan() { f32::NEG_INFINITY } else { l }))
        .collect();
    if pairs.len() > n {
        pairs.select_nth_unstable_by(n - 1, |a, b| b.1.total_cmp(&a.1));
        pairs.truncate(n);
    }
    pairs
}

/// Unnormalized softmax weight `exp(logit - max)`, robust to ±∞ logits.
fn softmax_weight(logit: f32, max_logit: f32) -> f32 {
    if max_logit == f32::INFINITY {
        (logit == f32::INFINITY) as u8 as f32
    } else if max_logit == f32::NEG_INFINITY || logit.is_nan() {
        0.0
    } else {
        (logit - max_logit).exp()
    }
}

/// Top-N candidates joined with their exact-cell presence and the presence masks of the
/// cell's TDWG/TEOW/FEOW/MEOW regions. Every subquery is a primary-key point lookup.
const CANDIDATE_SQL: &str = "
    SELECT t.ml_vision_id, t.taxon_id, t.scientific_name, t.common_name_ru,
           t.kingdom, t.safety_status, t.habitat_type,
           (SELECT g.months_bitmask FROM geo_presence AS g
             WHERE g.h3_index = ?2 AND g.taxon_id = t.taxon_id),
           (SELECT r.months_bitmask FROM taxon_regions AS r
             WHERE r.region_scheme = 0 AND r.region_code = ?3 AND r.taxon_id = t.taxon_id),
           (SELECT r.months_bitmask FROM taxon_regions AS r
             WHERE r.region_scheme = 1 AND r.region_code = ?4 AND r.taxon_id = t.taxon_id),
           (SELECT r.months_bitmask FROM taxon_regions AS r
             WHERE r.region_scheme = 2 AND r.region_code = ?5 AND r.taxon_id = t.taxon_id),
           (SELECT r.months_bitmask FROM taxon_regions AS r
             WHERE r.region_scheme = 3 AND r.region_code = ?6 AND r.taxon_id = t.taxon_id)
    FROM json_each(?1) AS j
    JOIN taxa AS t ON t.ml_vision_id = j.value";

pub struct Database {
    conn: Mutex<Connection>,
}

fn parse_kingdom(s: &str) -> Kingdom {
    match s {
        "Plantae" => Kingdom::Plantae,
        "Fungi" => Kingdom::Fungi,
        "Animalia" => Kingdom::Animalia,
        _ => Kingdom::Plantae,
    }
}

fn parse_safety(s: &str) -> SafetyStatus {
    match s {
        "Neutral" => SafetyStatus::Neutral,
        "Edible" => SafetyStatus::Edible,
        "Toxic" => SafetyStatus::Toxic,
        "Deadly" => SafetyStatus::Deadly,
        "Protected" => SafetyStatus::Protected,
        _ => SafetyStatus::Neutral,
    }
}

impl Database {
    /// Creates an in-memory database and seeds it with demo flora/fungi/fauna data.
    pub fn new_in_memory_demo() -> Result<Self, AtlasError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| AtlasError::DbError(e.to_string()))?;
        init_schema(&conn)?;
        dataset::seed_catalog(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, AtlasError> {
        self.conn
            .lock()
            .map_err(|_| AtlasError::DbError("database mutex poisoned".to_string()))
    }

    /// Ecoregions of an H3 cell, or `None` if the cell is not in `h3_geography`.
    pub fn lookup_ecoregions(&self, h3_index: i64) -> Result<Option<CellEcoregions>, AtlasError> {
        let conn = self.lock()?;
        query_cell_ecoregions(&conn, h3_index)
    }

    /// Taxa of one ecosystem, ordered by `taxon_id`.
    pub fn list_taxa_by_habitat(
        &self,
        habitat: HabitatType,
        limit: u32,
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare_cached(
                "SELECT taxon_id, scientific_name, common_name_ru, kingdom, safety_status
                 FROM taxa WHERE habitat_type = ?1 ORDER BY taxon_id LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![habitat.code(), limit], |row| {
                let k_str: String = row.get(3)?;
                let s_str: String = row.get(4)?;
                Ok(TaxonSummary {
                    taxon_id: row.get(0)?,
                    scientific_name: row.get(1)?,
                    common_name_ru: row.get(2)?,
                    kingdom: parse_kingdom(&k_str),
                    safety: parse_safety(&s_str),
                    confidence: 1.0,
                    geo_verified: false,
                })
            })
            .map_err(db_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_err)
    }

    pub fn get_taxon_habitat(&self, taxon_id: i64) -> Result<HabitatType, AtlasError> {
        let conn = self.lock()?;
        let code: Option<i64> = conn
            .prepare_cached("SELECT habitat_type FROM taxa WHERE taxon_id = ?1")
            .map_err(db_err)?
            .query_row(params![taxon_id], |row| row.get(0))
            .optional()
            .map_err(db_err)?;
        let code = code.ok_or(AtlasError::NotFound(taxon_id))?;
        HabitatType::from_code(code)
            .ok_or_else(|| AtlasError::DbError(format!("invalid habitat_type {code}")))
    }

    /// Opens an SQLite database from a file path.
    pub fn new_from_file(path: &str) -> Result<Self, AtlasError> {
        let conn = Connection::open(path)
            .map_err(|e| AtlasError::DbError(e.to_string()))?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Full-text search across scientific, Russian common names, and synonyms using SQLite FTS5.
    pub fn search_taxa(&self, query: &str, limit: u32) -> Result<Vec<TaxonSummary>, AtlasError> {
        let conn = self.conn.lock().unwrap();
        let clean_query = query.trim();

        if clean_query.is_empty() {
            let mut stmt = conn
                .prepare(
                    "SELECT taxon_id, scientific_name, common_name_ru, kingdom, safety_status
                     FROM taxa ORDER BY taxon_id LIMIT ?1",
                )
                .map_err(|e| AtlasError::DbError(e.to_string()))?;

            let rows = stmt
                .query_map(params![limit], |row| {
                    let k_str: String = row.get(3)?;
                    let s_str: String = row.get(4)?;
                    Ok(TaxonSummary {
                        taxon_id: row.get(0)?,
                        scientific_name: row.get(1)?,
                        common_name_ru: row.get(2)?,
                        kingdom: parse_kingdom(&k_str),
                        safety: parse_safety(&s_str),
                        confidence: 1.0,
                        geo_verified: false,
                    })
                })
                .map_err(|e| AtlasError::DbError(e.to_string()))?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| AtlasError::DbError(e.to_string()))?);
            }
            return Ok(results);
        }

        let sanitized: String = clean_query
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();

        let words: Vec<&str> = sanitized.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let fts_query = words
            .into_iter()
            .map(|w| format!("\"{}\"*", w))
            .collect::<Vec<_>>()
            .join(" ");

        let mut stmt = conn
            .prepare(
                "SELECT t.taxon_id, t.scientific_name, t.common_name_ru, t.kingdom, t.safety_status, bm25(taxa_fts) as rank
                 FROM taxa_fts fts
                 JOIN taxa t ON fts.rowid = t.taxon_id
                 WHERE taxa_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| AtlasError::DbError(e.to_string()))?;

        let rows = stmt
            .query_map(params![fts_query, limit], |row| {
                let k_str: String = row.get(3)?;
                let s_str: String = row.get(4)?;
                let rank: f64 = row.get(5)?;
                let conf = (1.0 / (1.0 + rank.abs())).clamp(0.1, 1.0) as f32;
                Ok(TaxonSummary {
                    taxon_id: row.get(0)?,
                    scientific_name: row.get(1)?,
                    common_name_ru: row.get(2)?,
                    kingdom: parse_kingdom(&k_str),
                    safety: parse_safety(&s_str),
                    confidence: conf,
                    geo_verified: false,
                })
            })
            .map_err(|e| AtlasError::DbError(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| AtlasError::DbError(e.to_string()))?);
        }
        Ok(results)
    }

    /// Retrieves detailed taxon information, decompressing the Zstd article.
    pub fn get_taxon_detail(&self, taxon_id: i64) -> Result<TaxonDetail, AtlasError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT taxon_id, scientific_name, common_name_ru, kingdom, safety_status, description_zstd
                 FROM taxa WHERE taxon_id = ?1",
            )
            .map_err(|e| AtlasError::DbError(e.to_string()))?;

        let mut row_iter = stmt
            .query_map(params![taxon_id], |row| {
                let k_str: String = row.get(3)?;
                let s_str: String = row.get(4)?;
                let desc_blob: Vec<u8> = row.get(5)?;
                Ok((
                    TaxonSummary {
                        taxon_id: row.get(0)?,
                        scientific_name: row.get(1)?,
                        common_name_ru: row.get(2)?,
                        kingdom: parse_kingdom(&k_str),
                        safety: parse_safety(&s_str),
                        confidence: 1.0,
                        geo_verified: false,
                    },
                    desc_blob,
                ))
            })
            .map_err(|e| AtlasError::DbError(e.to_string()))?;

        let (mut summary, desc_blob) = match row_iter.next() {
            Some(res) => res.map_err(|e| AtlasError::DbError(e.to_string()))?,
            None => return Err(AtlasError::NotFound(taxon_id)),
        };

        let decompressed_bytes = zstd::decode_all(&desc_blob[..])
            .map_err(|e| AtlasError::DecompressError(e.to_string()))?;
        let description_markdown = String::from_utf8(decompressed_bytes)
            .map_err(|e| AtlasError::DecompressError(e.to_string()))?;

        let mut h3_stmt = conn
            .prepare("SELECT h3_index FROM geo_presence WHERE taxon_id = ?1 LIMIT 1")
            .map_err(|e| AtlasError::DbError(e.to_string()))?;
        let h3_cell_hex: String = h3_stmt
            .query_row(params![taxon_id], |row| {
                let h3_idx: i64 = row.get(0)?;
                Ok(geo::h3_to_hex(h3_idx))
            })
            .unwrap_or_else(|_| "No GPS cell".to_string());

        if h3_cell_hex != "No GPS cell" {
            summary.geo_verified = true;
        }

        Ok(TaxonDetail {
            summary,
            description_markdown,
            h3_cell_hex,
        })
    }

    /// Applies Softmax, checks Uber H3 spatial presence + seasonal activity, penalizes non-matching taxa, and re-normalizes.
    pub fn postprocess_vision_logits(
        &self,
        logits: &[f32],
        lat: Option<f64>,
        lon: Option<f64>,
        month: u8,
        top_k: u32,
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        self.postprocess_vision_logits_in_habitats(logits, lat, lon, month, top_k, &[])
    }

    /// Spatio-temporal Bayesian re-ranking of classifier logits.
    ///
    /// 1. **Top-N pre-filter**: only the [`VISION_PREFILTER_TOP_N`] highest raw logits
    ///    are considered (partial selection in Rust), so the SQL work is constant for
    ///    any model size.
    /// 2. **One query** resolves those classes to taxa together with presence masks
    ///    from the exact H3 cell (`geo_presence`) and from the cell's TDWG/TEOW/FEOW/MEOW
    ///    regions (`h3_geography` → `taxon_regions`).
    /// 3. Optional habitat filter (`habitats` empty = keep all).
    /// 4. Softmax over the surviving candidates; taxa absent from the location or out of
    ///    season (by any matching source) get ×0.15; probabilities are re-normalized.
    ///
    /// Without coordinates no spatial penalty is applied. If several taxa share one
    /// `ml_vision_id`, the lowest `taxon_id` represents the class.
    pub fn postprocess_vision_logits_in_habitats(
        &self,
        logits: &[f32],
        lat: Option<f64>,
        lon: Option<f64>,
        month: u8,
        top_k: u32,
        habitats: &[HabitatType],
    ) -> Result<Vec<TaxonSummary>, AtlasError> {
        if logits.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }

        let top = top_n_logits(logits, VISION_PREFILTER_TOP_N);
        let max_logit = top.iter().map(|&(_, l)| l).fold(f32::NEG_INFINITY, f32::max);
        let mut ids_json = String::with_capacity(top.len() * 7 + 2);
        ids_json.push('[');
        for (i, (idx, _)) in top.iter().enumerate() {
            if i > 0 {
                ids_json.push(',');
            }
            ids_json.push_str(&idx.to_string());
        }
        ids_json.push(']');

        let target_h3 = match (lat, lon) {
            (Some(la), Some(lo)) => geo::lat_lon_to_h3_res4(la, lo),
            _ => None,
        };

        struct Row {
            vision_id: i64,
            habitat_code: i64,
            masks: [Option<i64>; 5],
            taxon: TaxonSummary,
        }

        let mut rows: Vec<Row> = {
            let conn = self.lock()?;
            let regions = match target_h3 {
                Some(h3) => query_cell_ecoregions(&conn, h3)?.unwrap_or_default(),
                None => CellEcoregions::default(),
            };
            let mut stmt = conn.prepare_cached(CANDIDATE_SQL).map_err(db_err)?;
            let mapped = stmt
                .query_map(
                    params![
                        ids_json,
                        target_h3,
                        regions.region(RegionScheme::Tdwg),
                        regions.region(RegionScheme::Teow),
                        regions.region(RegionScheme::Feow),
                        regions.region(RegionScheme::Meow),
                    ],
                    |row| {
                        let k_str: String = row.get(4)?;
                        let s_str: String = row.get(5)?;
                        Ok(Row {
                            vision_id: row.get(0)?,
                            habitat_code: row.get(6)?,
                            masks: [row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?],
                            taxon: TaxonSummary {
                                taxon_id: row.get(1)?,
                                scientific_name: row.get(2)?,
                                common_name_ru: row.get(3)?,
                                kingdom: parse_kingdom(&k_str),
                                safety: parse_safety(&s_str),
                                confidence: 0.0,
                                geo_verified: false,
                            },
                        })
                    },
                )
                .map_err(db_err)?;
            mapped.collect::<Result<_, _>>().map_err(db_err)?
        };

        rows.sort_by_key(|r| (r.vision_id, r.taxon.taxon_id));
        rows.dedup_by_key(|r| r.vision_id);

        let mut candidates: Vec<TaxonSummary> = Vec::with_capacity(rows.len());
        let mut weights: Vec<f32> = Vec::with_capacity(rows.len());
        for row in rows {
            if !habitats.is_empty() && !habitats.iter().any(|h| h.code() == row.habitat_code) {
                continue;
            }
            let logit = top
                .iter()
                .find(|&&(idx, _)| i64::from(idx) == row.vision_id)
                .map_or(f32::NEG_INFINITY, |&(_, l)| l);
            let mut weight = softmax_weight(logit, max_logit);
            let mut taxon = row.taxon;

            if target_h3.is_some() {
                let presence = row
                    .masks
                    .iter()
                    .flatten()
                    .fold(None, |acc: Option<u16>, &m| Some(acc.unwrap_or(0) | m as u16));
                match presence {
                    Some(mask) if geo::is_present_in_season(mask, month) => {
                        taxon.geo_verified = true;
                    }
                    // Out of season, or out of geography.
                    _ => weight *= GEO_PENALTY,
                }
            }
            candidates.push(taxon);
            weights.push(weight);
        }

        let total: f32 = weights.iter().sum();
        for (taxon, w) in candidates.iter_mut().zip(&weights) {
            taxon.confidence = if total > 0.0 { w / total } else { *w };
        }
        candidates.sort_by(|a, b| {
            b.confidence
                .total_cmp(&a.confidence)
                .then(a.taxon_id.cmp(&b.taxon_id))
        });
        candidates.truncate(top_k as usize);
        Ok(candidates)
    }
}

fn query_cell_ecoregions(
    conn: &Connection,
    h3_index: i64,
) -> Result<Option<CellEcoregions>, AtlasError> {
    conn.prepare_cached(
        "SELECT tdwg_code, teow_biome, feow_basin, meow_zone
         FROM h3_geography WHERE h3_index = ?1",
    )
    .map_err(db_err)?
    .query_row(params![h3_index], |row| {
        Ok(CellEcoregions {
            tdwg_code: row.get(0)?,
            teow_biome: row.get(1)?,
            feow_basin: row.get(2)?,
            meow_zone: row.get(3)?,
        })
    })
    .optional()
    .map_err(db_err)
}

/// Creates all tables and indexes (idempotent) and upgrades Stage 2A files in place by
/// adding `taxa.habitat_type` (defaulting to Terrestrial).
pub fn init_schema(conn: &Connection) -> Result<(), AtlasError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS taxa (
            taxon_id          INTEGER PRIMARY KEY,
            ml_vision_id      INTEGER,
            ml_audio_id       INTEGER,
            kingdom           TEXT NOT NULL CHECK(kingdom IN ('Plantae', 'Fungi', 'Animalia')),
            scientific_name   TEXT NOT NULL,
            common_name_ru    TEXT NOT NULL,
            synonyms          TEXT NOT NULL DEFAULT '',
            safety_status     TEXT NOT NULL CHECK(safety_status IN ('Neutral', 'Edible', 'Toxic', 'Deadly', 'Protected')),
            -- 0 = Terrestrial, 1 = Freshwater, 2 = Marine, 3 = Wetland
            habitat_type      INTEGER NOT NULL DEFAULT 0 CHECK(habitat_type BETWEEN 0 AND 3),
            description_zstd  BLOB NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS taxa_fts USING fts5(
            scientific_name,
            common_name_ru,
            synonyms,
            content='taxa',
            content_rowid='taxon_id',
            tokenize='unicode61 remove_diacritics 2'
        );

        CREATE TABLE IF NOT EXISTS geo_presence (
            h3_index        INTEGER NOT NULL,
            taxon_id        INTEGER NOT NULL REFERENCES taxa(taxon_id),
            months_bitmask  INTEGER NOT NULL,
            PRIMARY KEY (h3_index, taxon_id)
        );

        CREATE TABLE IF NOT EXISTS media_blobs (
            taxon_id    INTEGER PRIMARY KEY REFERENCES taxa(taxon_id),
            avif_bytes  BLOB NOT NULL
        );

        -- Planet-wide H3 (resolution 4) -> biogeographic regions. 0 = not applicable
        -- (e.g. no MEOW zone for an inland cell, no TDWG/TEOW for open sea).
        CREATE TABLE IF NOT EXISTS h3_geography (
            h3_index      INTEGER PRIMARY KEY,
            tdwg_code     INTEGER NOT NULL, -- Botanical country (TDWG Level 3, 1..369)
            teow_biome    INTEGER NOT NULL, -- Terrestrial ecoregion (TEOW, 1..867)
            feow_basin    INTEGER NOT NULL, -- Freshwater ecoregion (FEOW, 0..426)
            meow_zone     INTEGER NOT NULL  -- Marine ecoregion (MEOW, 0..232)
        ) WITHOUT ROWID;

        -- Species ranges by region: far more compact than per-cell presence for 50k taxa.
        CREATE TABLE IF NOT EXISTS taxon_regions (
            region_scheme   INTEGER NOT NULL CHECK(region_scheme BETWEEN 0 AND 3), -- 0 TDWG, 1 TEOW, 2 FEOW, 3 MEOW
            region_code     INTEGER NOT NULL CHECK(region_code > 0),
            taxon_id        INTEGER NOT NULL REFERENCES taxa(taxon_id),
            months_bitmask  INTEGER NOT NULL DEFAULT 4095,
            PRIMARY KEY (region_scheme, region_code, taxon_id)
        ) WITHOUT ROWID;
        ",
    )
    .map_err(db_err)?;

    if !table_has_column(conn, "taxa", "habitat_type")? {
        conn.execute_batch(
            "ALTER TABLE taxa ADD COLUMN habitat_type INTEGER NOT NULL DEFAULT 0
             CHECK(habitat_type BETWEEN 0 AND 3);",
        )
        .map_err(db_err)?;
    }

    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_taxa_ml_vision ON taxa(ml_vision_id);
        CREATE INDEX IF NOT EXISTS idx_taxa_ml_audio  ON taxa(ml_audio_id);
        CREATE INDEX IF NOT EXISTS idx_taxa_habitat   ON taxa(habitat_type);
        CREATE INDEX IF NOT EXISTS idx_geo_lookup     ON geo_presence(h3_index, months_bitmask);
        ",
    )
    .map_err(db_err)?;

    Ok(())
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, AtlasError> {
    conn.prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2")
        .and_then(|mut stmt| stmt.exists(params![table, column]))
        .map_err(db_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPB: (f64, f64) = (59.93, 30.33);
    const TOMSK: (f64, f64) = (56.48, 84.94);
    /// Number of classes in the demo catalog (`ml_vision_id` 0..=12).
    const DEMO_CLASSES: usize = 13;

    fn demo() -> Database {
        Database::new_in_memory_demo().expect("demo database")
    }

    fn one_hot(n: usize, hot: usize) -> Vec<f32> {
        let mut v = vec![0.0; n];
        v[hot] = 5.0;
        v
    }

    #[test]
    fn legacy_five_class_ranking_is_unchanged() {
        let db = demo();
        let logits = [3.0f32, 2.0, 1.0, 0.5, 0.0];

        let res = db
            .postprocess_vision_logits(&logits, Some(SPB.0), Some(SPB.1), 7, 3)
            .unwrap();
        assert_eq!(res.iter().map(|t| t.taxon_id).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(res.iter().all(|t| t.geo_verified));
        let denom: f32 = logits.iter().map(|l| l.exp()).sum();
        assert!((res[0].confidence - 3f32.exp() / denom).abs() < 1e-5);

        // Tomsk: no Amanita record there → ×0.15 drops it below the verified Boletus.
        let res = db
            .postprocess_vision_logits(&logits, Some(TOMSK.0), Some(TOMSK.1), 7, 5)
            .unwrap();
        assert_eq!(res.iter().map(|t| t.taxon_id).collect::<Vec<_>>(), [2, 1, 3, 5, 4]);
        assert!(!res[1].geo_verified && res[0].geo_verified);

        // No coordinates → plain softmax order, no spatial verification.
        let res = db.postprocess_vision_logits(&logits, None, None, 7, 5).unwrap();
        assert_eq!(res.iter().map(|t| t.taxon_id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        assert!(res.iter().all(|t| !t.geo_verified));

        assert!(db.postprocess_vision_logits(&[], None, None, 7, 5).unwrap().is_empty());
        assert!(db.postprocess_vision_logits(&logits, None, None, 7, 0).unwrap().is_empty());
    }

    #[test]
    fn habitat_type_filtering() {
        let db = demo();
        let names = |h: HabitatType| -> Vec<String> {
            db.list_taxa_by_habitat(h, 100)
                .unwrap()
                .into_iter()
                .map(|t| t.scientific_name)
                .collect()
        };
        assert_eq!(names(HabitatType::Marine), ["Zostera marina"]);
        assert_eq!(names(HabitatType::Freshwater), ["Nuphar lutea"]);
        assert_eq!(names(HabitatType::Wetland), ["Cicuta virosa", "Typha latifolia"]);
        let land = names(HabitatType::Terrestrial);
        assert_eq!(land.len(), DEMO_CLASSES - 4);
        assert!(land.iter().any(|n| n == "Sequoiadendron giganteum"));
        assert!(!land.iter().any(|n| n == "Nuphar lutea" || n == "Zostera marina"));

        assert_eq!(db.get_taxon_habitat(6).unwrap(), HabitatType::Wetland);
        assert_eq!(db.get_taxon_habitat(9).unwrap(), HabitatType::Marine);
        assert!(matches!(db.get_taxon_habitat(9_999), Err(AtlasError::NotFound(9_999))));

        // Pond shore near St Petersburg in July: equal logits for every class, but only
        // freshwater + wetland taxa may be returned.
        let logits = vec![1.0f32; DEMO_CLASSES];
        let aquatic = [HabitatType::Freshwater, HabitatType::Wetland];
        let res = db
            .postprocess_vision_logits_in_habitats(&logits, Some(SPB.0), Some(SPB.1), 7, 10, &aquatic)
            .unwrap();
        let mut ids: Vec<i64> = res.iter().map(|t| t.taxon_id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [6, 7, 8]);
        assert!(res.iter().all(|t| t.geo_verified), "verified via SPb TDWG/FEOW regions");
        let total: f32 = res.iter().map(|t| t.confidence).sum();
        assert!((total - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ecoregions_verify_presence_without_exact_cells() {
        let db = demo();
        let spb = geo::lat_lon_to_h3_res4(SPB.0, SPB.1).unwrap();
        let cell = db.lookup_ecoregions(spb).unwrap().expect("SPb anchor cell");
        assert_eq!(cell.tdwg_code, dataset::tdwg::RUW);
        assert_eq!(cell.feow_basin, dataset::feow::NORTHERN_BALTIC_DRAINAGES);
        assert_eq!(cell.meow_zone, dataset::meow::BALTIC_SEA);
        let pacific = geo::lat_lon_to_h3_res4(0.0, -150.0).unwrap();
        assert_eq!(db.lookup_ecoregions(pacific).unwrap(), None);

        // (vision class, (lat, lon), month, expected geo_verified)
        let cases = [
            (5, SPB, 7, true),                // Cicuta virosa: TDWG RUW, July
            (5, SPB, 1, false),               // …but out of season in January
            (5, (0.0, -150.0), 7, false),     // open Pacific: no geography row
            (8, (54.9, 8.35), 3, true),       // Zostera marina: North Sea (MEOW)
            (8, TOMSK, 7, false),             // …not in West Siberia
            (9, (-0.74, -90.31), 2, true),    // Hippomane mancinella: Galápagos
            (10, (-16.17, 145.42), 11, true), // Dendrocnide moroides: Queensland
            (11, (-2.98, 38.47), 8, true),    // Adansonia digitata: Kenya
            (12, (36.56, -118.77), 1, true),  // Sequoiadendron: Sierra Nevada
            (12, SPB, 7, false),
        ];
        for (class, (lat, lon), month, verified) in cases {
            let res = db
                .postprocess_vision_logits(&one_hot(DEMO_CLASSES, class), Some(lat), Some(lon), month, 1)
                .unwrap();
            assert_eq!(res[0].taxon_id, class as i64 + 1);
            assert_eq!(res[0].geo_verified, verified, "class {class} at ({lat}, {lon}) month {month}");
        }
    }

    #[test]
    fn top_n_logits_partial_selection() {
        let logits = [0.1, f32::NAN, 3.0, -1.0, 2.0, f32::INFINITY];
        let mut top = top_n_logits(&logits, 3);
        top.sort_by_key(|p| p.0);
        assert_eq!(top.iter().map(|p| p.0).collect::<Vec<_>>(), [2, 4, 5]);
        assert_eq!(top_n_logits(&logits, 10).len(), logits.len());
        assert!(top_n_logits(&logits, 0).is_empty());
        assert!(top_n_logits(&[f32::NAN; 4], 2).iter().all(|p| p.1 == f32::NEG_INFINITY));

        assert_eq!(softmax_weight(f32::INFINITY, f32::INFINITY), 1.0);
        assert_eq!(softmax_weight(3.0, f32::INFINITY), 0.0);
        assert_eq!(softmax_weight(f32::NEG_INFINITY, f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn top32_prefilter_at_planetary_scale() {
        const N: usize = 50_000;
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        {
            let tx = conn.unchecked_transaction().unwrap();
            {
                let mut ins = tx
                    .prepare(
                        "INSERT INTO taxa (taxon_id, ml_vision_id, kingdom, scientific_name,
                                           common_name_ru, safety_status, habitat_type, description_zstd)
                         VALUES (?1, ?2, 'Plantae', ?3, ?3, 'Neutral', ?4, x'00')",
                    )
                    .unwrap();
                for i in 0..N {
                    ins.execute(params![i as i64 + 1, i as i64, format!("Species {i}"), (i % 4) as i64])
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }

        // The candidate query must probe the ml_vision_id index, never scan 50k taxa.
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {CANDIDATE_SQL}"))
            .unwrap()
            .query_map(params!["[1,2,3]", 0i64, 1i64, 1i64, None::<i64>, None::<i64>], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(plan.iter().any(|d| d.contains("idx_taxa_ml_vision")), "{plan:?}");
        assert!(!plan.iter().any(|d| d.starts_with("SCAN t")), "{plan:?}");

        // Class 40_000 ranks 33rd by raw logit but is the only taxon present at SPb.
        // Without the pre-filter it would win: e^9 (verified) > e^10 × 0.15 (penalized).
        let spb = geo::lat_lon_to_h3_res4(SPB.0, SPB.1).unwrap();
        conn.execute(
            "INSERT INTO geo_presence (h3_index, taxon_id, months_bitmask) VALUES (?1, 40001, 4095)",
            params![spb],
        )
        .unwrap();
        let db = Database { conn: Mutex::new(conn) };

        let mut logits = vec![0.0f32; N];
        let top32: Vec<usize> = (0..32).map(|k| 1_000 + k * 1_499).collect();
        for (k, &i) in top32.iter().enumerate() {
            logits[i] = 10.0 - k as f32 * 0.01;
        }
        logits[40_000] = 9.0;

        let res = db
            .postprocess_vision_logits(&logits, Some(SPB.0), Some(SPB.1), 7, 5)
            .unwrap();
        let expected: Vec<i64> = top32[..5].iter().map(|&i| i as i64 + 1).collect();
        assert_eq!(res.iter().map(|t| t.taxon_id).collect::<Vec<_>>(), expected);
        assert!(res.iter().all(|t| t.taxon_id != 40_001 && !t.geo_verified));

        // Habitat filter acts on the same 32 candidates (indices ≡ 2 mod 4 are marine).
        let marine = db
            .postprocess_vision_logits_in_habitats(&logits, Some(SPB.0), Some(SPB.1), 7, 32, &[HabitatType::Marine])
            .unwrap();
        assert_eq!(marine.len(), top32.iter().filter(|&&i| i % 4 == 2).count());
        assert!(marine.iter().all(|t| (t.taxon_id - 1) % 4 == 2));

        const RUNS: u32 = 50;
        let t0 = std::time::Instant::now();
        for _ in 0..RUNS {
            db.postprocess_vision_logits(&logits, Some(SPB.0), Some(SPB.1), 7, 5).unwrap();
        }
        eprintln!(
            "postprocess_vision_logits, {N} logits: {:.3} ms/call",
            t0.elapsed().as_secs_f64() * 1e3 / RUNS as f64
        );
    }

    #[test]
    fn migrates_stage_2a_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE taxa (
                 taxon_id INTEGER PRIMARY KEY, ml_vision_id INTEGER, ml_audio_id INTEGER,
                 kingdom TEXT NOT NULL, scientific_name TEXT NOT NULL, common_name_ru TEXT NOT NULL,
                 synonyms TEXT NOT NULL DEFAULT '', safety_status TEXT NOT NULL,
                 description_zstd BLOB NOT NULL);
             INSERT INTO taxa VALUES (1, 0, NULL, 'Plantae', 'Vaccinium myrtillus', 'Черника', '', 'Edible', x'00');",
        )
        .unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap(); // idempotent
        let habitat: i64 = conn
            .query_row("SELECT habitat_type FROM taxa WHERE taxon_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(habitat, HabitatType::Terrestrial.code());
        assert!(table_has_column(&conn, "h3_geography", "meow_zone").unwrap());
        assert!(table_has_column(&conn, "taxon_regions", "region_scheme").unwrap());
    }
}
