use std::sync::Mutex;
use rusqlite::{params, Connection};

use crate::{
    geo, AtlasError, Kingdom, SafetyStatus, TaxonDetail, TaxonSummary,
};

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
        seed_demo_data(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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
        if logits.is_empty() {
            return Ok(Vec::new());
        }

        let max_logit = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp_sum: f32 = logits.iter().map(|l| (l - max_logit).exp()).sum();
        let probs: Vec<f32> = logits
            .iter()
            .map(|l| (l - max_logit).exp() / exp_sum)
            .collect();

        let conn = self.conn.lock().unwrap();

        let target_h3 = match (lat, lon) {
            (Some(la), Some(lo)) => geo::lat_lon_to_h3_res4(la, lo),
            _ => None,
        };

        struct Candidate {
            taxon: TaxonSummary,
            adjusted_prob: f32,
        }

        let mut candidates = Vec::new();

        for (idx, &prob) in probs.iter().enumerate() {
            let vision_id = idx as i64;
            let mut stmt = conn
                .prepare(
                    "SELECT taxon_id, scientific_name, common_name_ru, kingdom, safety_status
                     FROM taxa WHERE ml_vision_id = ?1",
                )
                .map_err(|e| AtlasError::DbError(e.to_string()))?;

            let mut rows = stmt
                .query_map(params![vision_id], |row| {
                    let k_str: String = row.get(3)?;
                    let s_str: String = row.get(4)?;
                    Ok(TaxonSummary {
                        taxon_id: row.get(0)?,
                        scientific_name: row.get(1)?,
                        common_name_ru: row.get(2)?,
                        kingdom: parse_kingdom(&k_str),
                        safety: parse_safety(&s_str),
                        confidence: prob,
                        geo_verified: false,
                    })
                })
                .map_err(|e| AtlasError::DbError(e.to_string()))?;

            if let Some(r) = rows.next() {
                let mut summary = r.map_err(|e| AtlasError::DbError(e.to_string()))?;
                let mut final_prob = prob;
                let mut verified = false;

                if let Some(h3) = target_h3 {
                    let mut geo_stmt = conn
                        .prepare(
                            "SELECT months_bitmask FROM geo_presence
                             WHERE h3_index = ?1 AND taxon_id = ?2",
                        )
                        .map_err(|e| AtlasError::DbError(e.to_string()))?;

                    let presence: Option<u16> = geo_stmt
                        .query_row(params![h3, summary.taxon_id], |row| {
                            let mask: i64 = row.get(0)?;
                            Ok(mask as u16)
                        })
                        .ok();

                    if let Some(mask) = presence {
                        if geo::is_present_in_season(mask, month) {
                            verified = true;
                        } else {
                            final_prob *= 0.15; // Out of season penalty
                        }
                    } else {
                        final_prob *= 0.15; // Out of geography penalty
                    }
                }

                summary.geo_verified = verified;
                candidates.push(Candidate {
                    taxon: summary,
                    adjusted_prob: final_prob,
                });
            }
        }

        let sum_adjusted: f32 = candidates.iter().map(|c| c.adjusted_prob).sum();
        for c in &mut candidates {
            if sum_adjusted > 0.0 {
                c.taxon.confidence = c.adjusted_prob / sum_adjusted;
            } else {
                c.taxon.confidence = c.adjusted_prob;
            }
        }

        candidates.sort_by(|a, b| {
            b.taxon
                .confidence
                .partial_cmp(&a.taxon.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let top = candidates
            .into_iter()
            .take(top_k as usize)
            .map(|c| c.taxon)
            .collect();

        Ok(top)
    }
}

fn init_schema(conn: &Connection) -> Result<(), AtlasError> {
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

        CREATE INDEX IF NOT EXISTS idx_taxa_ml_vision ON taxa(ml_vision_id);
        CREATE INDEX IF NOT EXISTS idx_taxa_ml_audio  ON taxa(ml_audio_id);
        CREATE INDEX IF NOT EXISTS idx_geo_lookup     ON geo_presence(h3_index, months_bitmask);
        ",
    )
    .map_err(|e| AtlasError::DbError(e.to_string()))?;

    Ok(())
}

fn seed_demo_data(conn: &Connection) -> Result<(), AtlasError> {
    let amanita_desc = "# Бледная поганка (*Amanita phalloides*)\n\nСмертельно ядовитый гриб из рода Мухомор (*Amanita*).\n\n## Описание и морфология\n* **Шляпка**: диаметром 5-15 см, оливковая, шелковистая, от бледно-зеленой до желтовато-бурой.\n* **Пластинки**: частые, белые, свободные от ножки.\n* **Ножка**: белая с муаровым рисунком, в основании клубневидное утолщение, заключенное в мешковидную белую вольву.\n* **Кольцо**: широкое, пленчатое, полосатое, свисающее.\n\n## Токсикология\nСодержит бициклические полипептиды: **аманитины** (аматоксины) и фаллоидины. Летальная доза — менее 30 г гриба. Токсины ингибируют РНК-полимеразу II, вызывая некроз печени и почек. Первые симптомы проявляются только через 6-24 часа!\n\n## Ареал\nЛиственные и смешанные леса, дубравы, березняки умеренной зоны Евразии.";

    let boletus_desc = "# Белый гриб (*Boletus edulis*)\n\nЦарь грибов, трубчатый съедобный гриб высшей (I) вкусовой категории.\n\n## Описание и морфология\n* **Шляпка**: диаметром 7-30 см, выпуклая, подушковидная, от светло-коричневой до каштановой.\n* **Трубчатый слой**: мелкие поры, сначала белые, затем желтоватые и оливково-зеленые.\n* **Мякоть**: плотная, белая, не изменяет цвет на срезе, с приятным ореховым ароматом.\n* **Ножка**: массивная, бочонковидная или булавовидная, со светлой сеточкой в верхней части.\n\n## Кулинарная ценность\nПревосходен во всех видах кулинарной обработки: сушка, жарка, варка, маринование. Не темнеет при сушке.";

    let blueberry_desc = "# Черника обыкновенная (*Vaccinium myrtillus*)\n\nНизкорослый кустарничек рода Вакциниум семейства Вересковые (*Ericaceae*).\n\n## Описание и морфология\n* **Стебель**: высотой 15-40 см, остроребристый, прямостоячий, зеленый.\n* **Листья**: мелкие, яйцевидные, очередные, осенью опадают.\n* **Плоды**: сочные черные с сизым налетом ягоды с темно-пурпурной красящей мякотью.\n\n## Польза и применение\nБогата антоцианами, антиоксидантами, витамином C и органическими кислотами. Традиционно применяется для улучшения ночного зрения и нормализации пищеварения.";

    let hogweed_desc = "# Борщевик Сосновского (*Heracleum sosnowskyi*)\n\nКрупное травянистое растение рода Борщевик семейства Зонтичные (*Apiaceae*). Опасный инвазивный вид!\n\n## Опасность для человека\nВсе части растения выделяют фуранокумарины (бергаптен, ксантотоксин). При попадании сока на кожу под воздействием солнечного ультрафиолета возникают тяжелейшие фитофотодерматиты — ожоги II-III степени с водянистыми волдырями.\n\n## Морфология\n* Гигантский стебель высотой до 3-5 метров, бороздчатый, полый.\n* Огромные тройчато-рассеченные листья длиной до 1-1.5 метра.\n* Соцветие — сложный зонтик диаметром до 50-80 см с сотнями мелких белых цветков.";

    let nightingale_desc = "# Обыкновенный соловей (*Luscinia luscinia*)\n\nПтица из семейства мухоловковых (*Muscicapidae*) отряда воробьинообразных. Знаменитый лесной певец.\n\n## Внешний вид\n* Оперение оливково-бурое сверху, беловато-серое снизу с неясной пятнистостью на груди.\n* Хвост с рыжеватым оттенком.\n* Размером с воробья (длина 16-19 см, вес 22-30 г).\n\n## Акустические особенности\nПесня сложная, звучная, состоит из четких повторяющихся колен: щелканье, свист, дроби («пульканье», «клыканье»). Поет преимущественно в сумерках и ночью в мае-июне.";

    let species = [
        (1i64, Some(0i64), None, "Fungi", "Amanita phalloides", "Бледная поганка", "мухомор зелёный, белая поганка, смертельный яд", "Deadly", amanita_desc),
        (2i64, Some(1i64), None, "Fungi", "Boletus edulis", "Белый гриб", "боровик, белевик, коровяк, глухарь", "Edible", boletus_desc),
        (3i64, Some(2i64), None, "Plantae", "Vaccinium myrtillus", "Черника обыкновенная", "черница, чернега, ягодник", "Edible", blueberry_desc),
        (4i64, Some(3i64), None, "Plantae", "Heracleum sosnowskyi", "Борщевик Сосновского", "гераклеум, ядовитый борщевик, ожог", "Toxic", hogweed_desc),
        (5i64, Some(4i64), Some(0i64), "Animalia", "Luscinia luscinia", "Обыкновенный соловей", "восточный соловей, соловейко, птица", "Neutral", nightingale_desc),
    ];

    for (taxon_id, ml_vis, ml_aud, kingdom, sci_name, com_name, synonyms, safety, desc_raw) in species {
        let compressed = zstd::encode_all(desc_raw.as_bytes(), 3)
            .map_err(|e| AtlasError::DbError(e.to_string()))?;

        conn.execute(
            "INSERT OR REPLACE INTO taxa
             (taxon_id, ml_vision_id, ml_audio_id, kingdom, scientific_name, common_name_ru, synonyms, safety_status, description_zstd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![taxon_id, ml_vis, ml_aud, kingdom, sci_name, com_name, synonyms, safety, compressed],
        )
        .map_err(|e| AtlasError::DbError(e.to_string()))?;

        conn.execute(
            "INSERT OR REPLACE INTO taxa_fts (rowid, scientific_name, common_name_ru, synonyms)
             VALUES (?1, ?2, ?3, ?4)",
            params![taxon_id, sci_name, com_name, synonyms],
        )
        .map_err(|e| AtlasError::DbError(e.to_string()))?;
    }

    // Coordinates:
    // St. Petersburg: 59.93, 30.33
    // Tomsk: 56.48, 84.94
    let spb_h3 = geo::lat_lon_to_h3_res4(59.93, 30.33).unwrap_or(0);
    let tomsk_h3 = geo::lat_lon_to_h3_res4(56.48, 84.94).unwrap_or(0);

    // Summer/autumn months: 6, 7, 8, 9, 10
    let summer_autumn_mask = geo::month_to_bitmask(6)
        | geo::month_to_bitmask(7)
        | geo::month_to_bitmask(8)
        | geo::month_to_bitmask(9)
        | geo::month_to_bitmask(10);

    // Bird song season: May, June, July, August (5, 6, 7, 8)
    let bird_mask = geo::month_to_bitmask(5)
        | geo::month_to_bitmask(6)
        | geo::month_to_bitmask(7)
        | geo::month_to_bitmask(8);

    let geo_records = [
        (spb_h3, 1i64, summer_autumn_mask),
        (spb_h3, 2i64, summer_autumn_mask),
        (spb_h3, 3i64, summer_autumn_mask),
        (spb_h3, 4i64, summer_autumn_mask),
        (spb_h3, 5i64, bird_mask),
        (tomsk_h3, 2i64, summer_autumn_mask),
        (tomsk_h3, 3i64, summer_autumn_mask),
        (tomsk_h3, 5i64, bird_mask),
    ];

    for (h3_idx, taxon_id, mask) in geo_records {
        conn.execute(
            "INSERT OR REPLACE INTO geo_presence (h3_index, taxon_id, months_bitmask)
             VALUES (?1, ?2, ?3)",
            params![h3_idx, taxon_id, mask as i64],
        )
        .map_err(|e| AtlasError::DbError(e.to_string()))?;
    }

    Ok(())
}
