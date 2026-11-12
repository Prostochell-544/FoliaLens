//! Curated starter catalog for the offline atlas: multi-biome species from every
//! inhabited continent (including freshwater, wetland and marine markers), their
//! biogeographic range bindings, and default ecoregions for key H3 cells of the world.
//!
//! Used by both [`crate::db::Database::new_in_memory_demo`] and the `build-atlas`
//! binary that produces `app/src/main/assets/atlas.db`.
//!
//! ## Region ids
//! `h3_geography` and `taxon_regions` store regions as integers per scheme. The ids
//! below are a **provisional project registry** (`1..` in each scheme, `0` = not
//! applicable) covering the regions this catalog uses; [`REGIONS`] maps each id to the
//! official TDWG Level 3 code or the official TEOW / FEOW / MEOW ecoregion name. A full
//! import from the official shapefiles must generate the complete registry and replace
//! these ids consistently in both tables.

use h3o::{LatLng, Resolution};
use rusqlite::{params, Connection};

use crate::db::{CellEcoregions, RegionScheme};
use crate::{geo, AtlasError, HabitatType};

/// TDWG / WGSRPD Level 3 botanical countries.
pub mod tdwg {
    pub const RUW: i64 = 1;
    pub const RUC: i64 = 2;
    pub const RUN: i64 = 3;
    pub const WSB: i64 = 4;
    pub const GER: i64 = 5;
    pub const CAL: i64 = 6;
    pub const FLA: i64 = 7;
    pub const PUE: i64 = 8;
    pub const GAL: i64 = 9;
    pub const QLD: i64 = 10;
    pub const NSW: i64 = 11;
    pub const KEN: i64 = 12;
    pub const TVL: i64 = 13;
}

/// WWF Terrestrial Ecoregions of the World.
pub mod teow {
    pub const SARMATIC_MIXED_FORESTS: i64 = 1;
    pub const SCANDINAVIAN_AND_RUSSIAN_TAIGA: i64 = 2;
    pub const WEST_SIBERIAN_TAIGA: i64 = 3;
    pub const ATLANTIC_MIXED_FORESTS: i64 = 4;
    pub const SIERRA_NEVADA_FORESTS: i64 = 5;
    pub const NORTHERN_CALIFORNIA_COASTAL_FORESTS: i64 = 6;
    pub const EVERGLADES: i64 = 7;
    pub const PUERTO_RICAN_DRY_FORESTS: i64 = 8;
    pub const GALAPAGOS_SCRUBLAND_MOSAIC: i64 = 9;
    pub const QUEENSLAND_TROPICAL_RAIN_FORESTS: i64 = 10;
    pub const NORTHERN_ACACIA_COMMIPHORA_BUSHLANDS: i64 = 11;
    pub const ZAMBEZIAN_AND_MOPANE_WOODLANDS: i64 = 12;
}

/// Freshwater Ecoregions of the World.
pub mod feow {
    pub const NORTHERN_BALTIC_DRAINAGES: i64 = 1;
    pub const VOLGA_URAL: i64 = 2;
    pub const UPPER_OB: i64 = 3;
    pub const CENTRAL_AND_WESTERN_EUROPE: i64 = 4;
    pub const SACRAMENTO_SAN_JOAQUIN: i64 = 5;
    pub const FLORIDA_PENINSULA: i64 = 6;
}

/// Marine Ecoregions of the World.
pub mod meow {
    pub const BALTIC_SEA: i64 = 1;
    pub const NORTH_SEA: i64 = 2;
    pub const WHITE_SEA: i64 = 3;
    pub const NORTHERN_CALIFORNIA: i64 = 4;
    pub const FLORIDIAN: i64 = 5;
    pub const GREATER_ANTILLES: i64 = 6;
    pub const EASTERN_GALAPAGOS_ISLANDS: i64 = 7;
}

/// Registry entry: provisional id → official code (TDWG only) and name.
#[derive(Debug, Clone, Copy)]
pub struct RegionInfo {
    pub scheme: RegionScheme,
    pub id: i64,
    /// Official TDWG Level 3 code; empty for ecoregion schemes, which are named.
    pub code: &'static str,
    pub name: &'static str,
}

const fn region(scheme: RegionScheme, id: i64, code: &'static str, name: &'static str) -> RegionInfo {
    RegionInfo { scheme, id, code, name }
}

use RegionScheme::{Feow, Meow, Tdwg, Teow};

pub const REGIONS: &[RegionInfo] = &[
    region(Tdwg, tdwg::RUW, "RUW", "Northwest European Russia"),
    region(Tdwg, tdwg::RUC, "RUC", "Central European Russia"),
    region(Tdwg, tdwg::RUN, "RUN", "North European Russia"),
    region(Tdwg, tdwg::WSB, "WSB", "West Siberia"),
    region(Tdwg, tdwg::GER, "GER", "Germany"),
    region(Tdwg, tdwg::CAL, "CAL", "California"),
    region(Tdwg, tdwg::FLA, "FLA", "Florida"),
    region(Tdwg, tdwg::PUE, "PUE", "Puerto Rico"),
    region(Tdwg, tdwg::GAL, "GAL", "Galápagos"),
    region(Tdwg, tdwg::QLD, "QLD", "Queensland"),
    region(Tdwg, tdwg::NSW, "NSW", "New South Wales"),
    region(Tdwg, tdwg::KEN, "KEN", "Kenya"),
    region(Tdwg, tdwg::TVL, "TVL", "Northern Provinces (South Africa)"),
    region(Teow, teow::SARMATIC_MIXED_FORESTS, "", "Sarmatic mixed forests"),
    region(Teow, teow::SCANDINAVIAN_AND_RUSSIAN_TAIGA, "", "Scandinavian and Russian taiga"),
    region(Teow, teow::WEST_SIBERIAN_TAIGA, "", "West Siberian taiga"),
    region(Teow, teow::ATLANTIC_MIXED_FORESTS, "", "Atlantic mixed forests"),
    region(Teow, teow::SIERRA_NEVADA_FORESTS, "", "Sierra Nevada forests"),
    region(Teow, teow::NORTHERN_CALIFORNIA_COASTAL_FORESTS, "", "Northern California coastal forests"),
    region(Teow, teow::EVERGLADES, "", "Everglades"),
    region(Teow, teow::PUERTO_RICAN_DRY_FORESTS, "", "Puerto Rican dry forests"),
    region(Teow, teow::GALAPAGOS_SCRUBLAND_MOSAIC, "", "Galápagos Islands scrubland mosaic"),
    region(Teow, teow::QUEENSLAND_TROPICAL_RAIN_FORESTS, "", "Queensland tropical rain forests"),
    region(Teow, teow::NORTHERN_ACACIA_COMMIPHORA_BUSHLANDS, "", "Northern Acacia-Commiphora bushlands and thickets"),
    region(Teow, teow::ZAMBEZIAN_AND_MOPANE_WOODLANDS, "", "Zambezian and mopane woodlands"),
    region(Feow, feow::NORTHERN_BALTIC_DRAINAGES, "", "Northern Baltic Drainages"),
    region(Feow, feow::VOLGA_URAL, "", "Volga - Ural"),
    region(Feow, feow::UPPER_OB, "", "Upper Ob"),
    region(Feow, feow::CENTRAL_AND_WESTERN_EUROPE, "", "Central & Western Europe"),
    region(Feow, feow::SACRAMENTO_SAN_JOAQUIN, "", "Sacramento - San Joaquin"),
    region(Feow, feow::FLORIDA_PENINSULA, "", "Florida Peninsula"),
    region(Meow, meow::BALTIC_SEA, "", "Baltic Sea"),
    region(Meow, meow::NORTH_SEA, "", "North Sea"),
    region(Meow, meow::WHITE_SEA, "", "White Sea"),
    region(Meow, meow::NORTHERN_CALIFORNIA, "", "Northern California"),
    region(Meow, meow::FLORIDIAN, "", "Floridian"),
    region(Meow, meow::GREATER_ANTILLES, "", "Greater Antilles"),
    region(Meow, meow::EASTERN_GALAPAGOS_ISLANDS, "", "Eastern Galapagos Islands"),
];

/// 12-bit month mask for an inclusive range of 1-based months.
const fn months(from: u8, to: u8) -> u16 {
    let mut mask = 0u16;
    let mut m = from;
    while m <= to {
        mask |= 1 << (m - 1);
        m += 1;
    }
    mask
}

const ALL_YEAR: u16 = months(1, 12);

/// One catalog species.
#[derive(Debug, Clone, Copy)]
pub struct SpeciesSeed {
    pub taxon_id: i64,
    pub ml_vision_id: Option<i64>,
    pub ml_audio_id: Option<i64>,
    pub kingdom: &'static str,
    pub scientific_name: &'static str,
    pub common_name_ru: &'static str,
    pub synonyms: &'static str,
    pub safety_status: &'static str,
    pub habitat: HabitatType,
    pub description_md: &'static str,
    /// Months the taxon is identifiable within its regions (12-bit mask).
    pub region_months: u16,
    /// Range as `(scheme, region id)` pairs — written to `taxon_regions`.
    pub regions: &'static [(RegionScheme, i64)],
}

const AMANITA_DESC: &str = "# Бледная поганка (*Amanita phalloides*)\n\nСмертельно ядовитый гриб из рода Мухомор (*Amanita*).\n\n## Описание и морфология\n* **Шляпка**: диаметром 5-15 см, оливковая, шелковистая, от бледно-зеленой до желтовато-бурой.\n* **Пластинки**: частые, белые, свободные от ножки.\n* **Ножка**: белая с муаровым рисунком, в основании клубневидное утолщение, заключенное в мешковидную белую вольву.\n* **Кольцо**: широкое, пленчатое, полосатое, свисающее.\n\n## Токсикология\nСодержит бициклические полипептиды: **аманитины** (аматоксины) и фаллоидины. Летальная доза — менее 30 г гриба. Токсины ингибируют РНК-полимеразу II, вызывая некроз печени и почек. Первые симптомы проявляются только через 6-24 часа!\n\n## Ареал\nЛиственные и смешанные леса, дубравы, березняки умеренной зоны Евразии.";

const BOLETUS_DESC: &str = "# Белый гриб (*Boletus edulis*)\n\nЦарь грибов, трубчатый съедобный гриб высшей (I) вкусовой категории.\n\n## Описание и морфология\n* **Шляпка**: диаметром 7-30 см, выпуклая, подушковидная, от светло-коричневой до каштановой.\n* **Трубчатый слой**: мелкие поры, сначала белые, затем желтоватые и оливково-зеленые.\n* **Мякоть**: плотная, белая, не изменяет цвет на срезе, с приятным ореховым ароматом.\n* **Ножка**: массивная, бочонковидная или булавовидная, со светлой сеточкой в верхней части.\n\n## Кулинарная ценность\nПревосходен во всех видах кулинарной обработки: сушка, жарка, варка, маринование. Не темнеет при сушке.";

const BLUEBERRY_DESC: &str = "# Черника обыкновенная (*Vaccinium myrtillus*)\n\nНизкорослый кустарничек рода Вакциниум семейства Вересковые (*Ericaceae*).\n\n## Описание и морфология\n* **Стебель**: высотой 15-40 см, остроребристый, прямостоячий, зеленый.\n* **Листья**: мелкие, яйцевидные, очередные, осенью опадают.\n* **Плоды**: сочные черные с сизым налетом ягоды с темно-пурпурной красящей мякотью.\n\n## Польза и применение\nБогата антоцианами, антиоксидантами, витамином C и органическими кислотами. Традиционно применяется для улучшения ночного зрения и нормализации пищеварения.";

const HOGWEED_DESC: &str = "# Борщевик Сосновского (*Heracleum sosnowskyi*)\n\nКрупное травянистое растение рода Борщевик семейства Зонтичные (*Apiaceae*). Опасный инвазивный вид!\n\n## Опасность для человека\nВсе части растения выделяют фуранокумарины (бергаптен, ксантотоксин). При попадании сока на кожу под воздействием солнечного ультрафиолета возникают тяжелейшие фитофотодерматиты — ожоги II-III степени с водянистыми волдырями.\n\n## Морфология\n* Гигантский стебель высотой до 3-5 метров, бороздчатый, полый.\n* Огромные тройчато-рассеченные листья длиной до 1-1.5 метра.\n* Соцветие — сложный зонтик диаметром до 50-80 см с сотнями мелких белых цветков.";

const NIGHTINGALE_DESC: &str = "# Обыкновенный соловей (*Luscinia luscinia*)\n\nПтица из семейства мухоловковых (*Muscicapidae*) отряда воробьинообразных. Знаменитый лесной певец.\n\n## Внешний вид\n* Оперение оливково-бурое сверху, беловато-серое снизу с неясной пятнистостью на груди.\n* Хвост с рыжеватым оттенком.\n* Размером с воробья (длина 16-19 см, вес 22-30 г).\n\n## Акустические особенности\nПесня сложная, звучная, состоит из четких повторяющихся колен: щелканье, свист, дроби («пульканье», «клыканье»). Поет преимущественно в сумерках и ночью в мае-июне.";

const CICUTA_DESC: &str = "# Вех ядовитый (*Cicuta virosa*)\n\nМноголетнее травянистое растение семейства Зонтичные (*Apiaceae*), одно из самых ядовитых растений умеренной Евразии.\n\n## Описание и морфология\n* **Корневище**: толстое, вертикальное, белое; на продольном разрезе видны **поперечные полости-камеры**, заполненные желтоватым соком, — главный диагностический признак.\n* **Стебель**: высотой 50–150 см, полый, гладкий, ветвистый, без пятен.\n* **Листья**: дважды-трижды перисторассечённые, с узкими линейно-ланцетными пильчатыми долями.\n* **Соцветие**: сложный зонтик из 10–20 лучей с мелкими белыми цветками; цветёт в июле–августе.\n* **Запах**: растение пахнет сельдереем или морковью, поэтому его путают со съедобными зонтичными.\n\n## Токсикология\nСодержит **цикутотоксин** — полиацетиленовый спирт, блокирующий ГАМК-рецепторы центральной нервной системы. Симптомы появляются в течение 15–60 минут: рвота, судороги, остановка дыхания. Сильнее всего ядовито корневище; смертельно опасен даже небольшой его кусочек. При подозрении на отравление немедленно вызывайте скорую помощь.\n\n## Экология и ареал\nГелофит: растёт по топким берегам рек, озёр и прудов, на болотах и в канавах, часто прямо в воде на мелководье. Распространён в умеренной Евразии от Западной Европы до Дальнего Востока.";

const NUPHAR_DESC: &str = "# Кубышка жёлтая (*Nuphar lutea*)\n\nМноголетнее водное растение семейства Кувшинковые (*Nymphaeaceae*).\n\n## Описание и морфология\n* **Корневище**: толстое (5–10 см), ползучее по дну, с рубцами отмерших листьев.\n* **Листья**: плавающие — кожистые, овально-сердцевидные, до 30 см длиной; подводные — тонкие, полупрозрачные, волнистые.\n* **Цветки**: жёлтые, шаровидные, 4–6 см в диаметре, приподняты над водой, со сладковатым «винным» запахом. Цветёт в июне–августе.\n* **Плод**: зелёный, кувшинообразный, многосемянный — отсюда название «кубышка».\n\n## Безопасность\nКорневища и семена содержат алкалоиды (нуфарин и др.). Растение считается ядовитым: не используйте его в пищу и для самолечения.\n\n## Экология и ареал\nГидрофит стоячих и медленно текущих водоёмов глубиной 0,5–3 м: озёра, старицы, пруды, речные заводи. Европа, Западная Азия, Сибирь, Северная Африка.";

const TYPHA_DESC: &str = "# Рогоз широколистный (*Typha latifolia*)\n\nКрупный многолетний гелофит семейства Рогозовые (*Typhaceae*). В быту его часто ошибочно называют «камышом».\n\n## Описание и морфология\n* **Высота**: 1–2,5 м; мощное ползучее корневище.\n* **Листья**: линейные, плоские, сизовато-зелёные, шириной 1–2 см.\n* **Соцветие**: плотный тёмно-коричневый «початок» (женская часть) длиной 10–20 см; мужская часть расположена **сразу над ним, без промежутка** — этим вид отличается от рогоза узколистного (*T. angustifolia*), у которого между ними есть разрыв.\n\n## Использование\nКорневища богаты крахмалом и съедобны после термической обработки, молодые побеги едят сырыми. **Внимание**: собирайте только в чистых водоёмах и только при уверенном определении — рядом нередко растёт смертельно ядовитый вех (*Cicuta virosa*).\n\n## Экология и ареал\nМелководья озёр и прудов, болота, канавы, берега рек. Почти космополит: Евразия, Северная Америка, Африка.";

const ZOSTERA_DESC: &str = "# Взморник морской (*Zostera marina*)\n\nМорское цветковое растение семейства Взморниковые (*Zosteraceae*) — одна из немногих цветковых трав, проводящих всю жизнь в солёной воде.\n\n## Описание и морфология\n* **Листья**: лентовидные, ярко-зелёные, длиной 20–120 см и шириной 2–10 мм, с параллельными жилками.\n* **Корневище**: ползучее, укореняется в песчаном или илистом грунте.\n* **Цветение**: под водой; пыльца переносится течением.\n\n## Экологическое значение\nОбразует подводные «луга» на глубинах до 5–10 м: убежище для молоди рыб и беспозвоночных, нерестилище сельди, корм чёрной казарки. Стабилизирует донные осадки и накапливает углерод. В 1930-х годах луга Северной Атлантики массово погибли от «болезни истощения», вызванной протистом *Labyrinthula zosterae*.\n\n## Ареал\nУмеренные и холодные моря Северного полушария: Северная Атлантика, Балтийское, Белое и Северное моря, север Тихого океана.";

const HIPPOMANE_DESC: &str = "# Манцинелловое дерево (*Hippomane mancinella*)\n\nВечнозелёное дерево семейства Молочайные (*Euphorbiaceae*), которое Книга рекордов Гиннесса называет самым опасным деревом в мире. Испанское название — *manzanilla de la muerte* («яблочко смерти»).\n\n## Описание и морфология\n* **Размер**: до 15 м, кора серо-коричневая.\n* **Листья**: блестящие, яйцевидные, мелкопильчатые, 5–10 см.\n* **Плоды**: зелёные или желтоватые, похожи на маленькие яблоки и сладковаты на вкус.\n\n## Опасность\nВсе части дерева содержат едкий млечный сок с **форболовыми эфирами**: при контакте — химические ожоги кожи и глаз вплоть до временной слепоты. **Не прячьтесь под деревом во время дождя**, не жгите его древесину (дым поражает глаза и дыхательные пути) и не пробуйте плоды — они вызывают тяжёлые ожоги рта, горла и пищеварительного тракта. На многих пляжах такие деревья помечают красными кольцами или табличками.\n\n## Ареал\nПесчаные побережья и окраины мангров Неотропики: Флорида, Карибские острова, Мексика, Центральная Америка, север Южной Америки, Галапагосские острова.";

const DENDROCNIDE_DESC: &str = "# Гимпи-гимпи (*Dendrocnide moroides*)\n\nЖалящее растение семейства Крапивные (*Urticaceae*) из влажных тропических лесов Австралии — одно из самых болезненных растений на Земле.\n\n## Описание и морфология\n* **Форма**: кустарник или небольшое деревце 1–5 м; часто растёт на просветах, опушках и вырубках.\n* **Листья**: крупные, сердцевидные, 12–22 см, с зубчатым краем; всё растение покрыто полыми **стрекательными волосками** с кремнезёмными кончиками.\n* **Плоды**: мелкие, сочные, розово-красные, похожие на ягоды малины или шелковицы.\n\n## Опасность\nВолоски впрыскивают нейротоксичные пептиды **гимпиетиды** (*gympietides*). Жгучая боль длится часами, а повторяющиеся приступы — неделями и даже месяцами; волоски почти невозможно извлечь из кожи. Не касайтесь даже засохших листьев и гербарных образцов: волоски сохраняют активность годами.\n\n## Ареал\nВлажные тропические леса северо-востока Квинсленда, север Нового Южного Уэльса, Молуккские острова.";

const ADANSONIA_DESC: &str = "# Баобаб африканский (*Adansonia digitata*)\n\nЛистопадное дерево семейства Мальвовые (*Malvaceae*), символ африканской саванны.\n\n## Описание и морфология\n* **Ствол**: бочковидный, диаметром до 10 м и более, при высоте 5–25 м; мягкая волокнистая древесина запасает тысячи литров воды.\n* **Листья**: пальчатосложные, из 5–7 листочков, опадают в сухой сезон.\n* **Цветки**: крупные, белые, повислые; раскрываются ночью и опыляются летучими мышами.\n* **Плод**: продолговатая коробочка до 25 см с бархатистой оболочкой («обезьяний хлеб»).\n\n## Использование\nСухая мякоть плода съедобна, кисловата и богата витамином C, кальцием и клетчаткой; молодые листья едят как овощ, из семян получают масло.\n\n## Долголетие\nОдно из самых долгоживущих цветковых растений: радиоуглеродное датирование показало возраст отдельных деревьев более 2000 лет. С 2005 года погибли многие из самых старых и крупных баобабов.\n\n## Ареал\nСухие саванны и редколесья тропической Африки к югу от Сахары.";

const SEQUOIADENDRON_DESC: &str = "# Секвойядендрон гигантский (*Sequoiadendron giganteum*)\n\nВечнозелёное хвойное дерево семейства Кипарисовые (*Cupressaceae*), «мамонтово дерево» — самое массивное дерево на Земле.\n\n## Описание и морфология\n* **Размер**: высота до 95 м, диаметр ствола у основания до 8–11 м; объём ствола «Генерала Шермана» — около 1 487 м³.\n* **Кора**: красно-коричневая, волокнистая, толщиной до 60–90 см; защищает от низовых пожаров.\n* **Хвоя**: шиловидная, плотно прижатая к побегам.\n* **Шишки**: яйцевидные, 5–8 см; многие раскрываются только после нагрева пожаром.\n\n## Возраст и охрана\nОтдельные деревья живут более 3000 лет. В Красном списке МСОП вид числится как находящийся под угрозой исчезновения (EN): по оценкам, пожары 2020–2021 годов уничтожили 13–19 % крупных деревьев.\n\n## Ареал\nОколо 70 рощ на западных склонах Сьерра-Невады (Калифорния, США) на высотах 1 400–2 150 м.";

/// Starter catalog. Taxa 1–5 are the Stage 2A demo set (ids and model indices are stable).
pub const CATALOG: &[SpeciesSeed] = &[
    SpeciesSeed {
        taxon_id: 1,
        ml_vision_id: Some(0),
        ml_audio_id: None,
        kingdom: "Fungi",
        scientific_name: "Amanita phalloides",
        common_name_ru: "Бледная поганка",
        synonyms: "мухомор зелёный, белая поганка, смертельный яд",
        safety_status: "Deadly",
        habitat: HabitatType::Terrestrial,
        description_md: AMANITA_DESC,
        region_months: 0,
        regions: &[],
    },
    SpeciesSeed {
        taxon_id: 2,
        ml_vision_id: Some(1),
        ml_audio_id: None,
        kingdom: "Fungi",
        scientific_name: "Boletus edulis",
        common_name_ru: "Белый гриб",
        synonyms: "боровик, белевик, коровяк, глухарь",
        safety_status: "Edible",
        habitat: HabitatType::Terrestrial,
        description_md: BOLETUS_DESC,
        region_months: 0,
        regions: &[],
    },
    SpeciesSeed {
        taxon_id: 3,
        ml_vision_id: Some(2),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Vaccinium myrtillus",
        common_name_ru: "Черника обыкновенная",
        synonyms: "черница, чернега, ягодник",
        safety_status: "Edible",
        habitat: HabitatType::Terrestrial,
        description_md: BLUEBERRY_DESC,
        region_months: 0,
        regions: &[],
    },
    SpeciesSeed {
        taxon_id: 4,
        ml_vision_id: Some(3),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Heracleum sosnowskyi",
        common_name_ru: "Борщевик Сосновского",
        synonyms: "гераклеум, ядовитый борщевик, ожог",
        safety_status: "Toxic",
        habitat: HabitatType::Terrestrial,
        description_md: HOGWEED_DESC,
        region_months: 0,
        regions: &[],
    },
    SpeciesSeed {
        taxon_id: 5,
        ml_vision_id: Some(4),
        ml_audio_id: Some(0),
        kingdom: "Animalia",
        scientific_name: "Luscinia luscinia",
        common_name_ru: "Обыкновенный соловей",
        synonyms: "восточный соловей, соловейко, птица",
        safety_status: "Neutral",
        habitat: HabitatType::Terrestrial,
        description_md: NIGHTINGALE_DESC,
        region_months: 0,
        regions: &[],
    },
    SpeciesSeed {
        taxon_id: 6,
        ml_vision_id: Some(5),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Cicuta virosa",
        common_name_ru: "Вех ядовитый",
        synonyms: "цикута, водяной болиголов, омег водяной, кошачья петрушка",
        safety_status: "Deadly",
        habitat: HabitatType::Wetland,
        description_md: CICUTA_DESC,
        region_months: months(5, 10),
        regions: &[
            (Tdwg, tdwg::RUW),
            (Tdwg, tdwg::RUC),
            (Tdwg, tdwg::RUN),
            (Tdwg, tdwg::WSB),
            (Tdwg, tdwg::GER),
            (Feow, feow::NORTHERN_BALTIC_DRAINAGES),
            (Feow, feow::VOLGA_URAL),
            (Feow, feow::UPPER_OB),
            (Feow, feow::CENTRAL_AND_WESTERN_EUROPE),
        ],
    },
    SpeciesSeed {
        taxon_id: 7,
        ml_vision_id: Some(6),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Nuphar lutea",
        common_name_ru: "Кубышка жёлтая",
        synonyms: "жёлтая кувшинка, водяная кубышка, кувшинка жёлтая",
        safety_status: "Toxic",
        habitat: HabitatType::Freshwater,
        description_md: NUPHAR_DESC,
        region_months: months(5, 10),
        regions: &[
            (Tdwg, tdwg::RUW),
            (Tdwg, tdwg::RUC),
            (Tdwg, tdwg::WSB),
            (Tdwg, tdwg::GER),
            (Feow, feow::NORTHERN_BALTIC_DRAINAGES),
            (Feow, feow::VOLGA_URAL),
            (Feow, feow::UPPER_OB),
            (Feow, feow::CENTRAL_AND_WESTERN_EUROPE),
        ],
    },
    SpeciesSeed {
        taxon_id: 8,
        ml_vision_id: Some(7),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Typha latifolia",
        common_name_ru: "Рогоз широколистный",
        synonyms: "камыш, куга, чакан, рогоз",
        safety_status: "Edible",
        habitat: HabitatType::Wetland,
        description_md: TYPHA_DESC,
        region_months: ALL_YEAR,
        regions: &[
            (Tdwg, tdwg::RUW),
            (Tdwg, tdwg::RUC),
            (Tdwg, tdwg::WSB),
            (Tdwg, tdwg::GER),
            (Tdwg, tdwg::CAL),
            (Tdwg, tdwg::FLA),
            (Feow, feow::NORTHERN_BALTIC_DRAINAGES),
            (Feow, feow::VOLGA_URAL),
            (Feow, feow::UPPER_OB),
            (Feow, feow::CENTRAL_AND_WESTERN_EUROPE),
            (Feow, feow::SACRAMENTO_SAN_JOAQUIN),
            (Feow, feow::FLORIDA_PENINSULA),
        ],
    },
    SpeciesSeed {
        taxon_id: 9,
        ml_vision_id: Some(8),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Zostera marina",
        common_name_ru: "Взморник морской",
        synonyms: "зостера, морская трава, взморник",
        safety_status: "Neutral",
        habitat: HabitatType::Marine,
        description_md: ZOSTERA_DESC,
        region_months: ALL_YEAR,
        regions: &[
            (Meow, meow::BALTIC_SEA),
            (Meow, meow::NORTH_SEA),
            (Meow, meow::WHITE_SEA),
            (Meow, meow::NORTHERN_CALIFORNIA),
        ],
    },
    SpeciesSeed {
        taxon_id: 10,
        ml_vision_id: Some(9),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Hippomane mancinella",
        common_name_ru: "Манцинелловое дерево",
        synonyms: "манцинелла, яблоко смерти, манчинил",
        safety_status: "Deadly",
        habitat: HabitatType::Terrestrial,
        description_md: HIPPOMANE_DESC,
        region_months: ALL_YEAR,
        regions: &[
            (Tdwg, tdwg::FLA),
            (Tdwg, tdwg::PUE),
            (Tdwg, tdwg::GAL),
            (Teow, teow::EVERGLADES),
            (Teow, teow::PUERTO_RICAN_DRY_FORESTS),
            (Teow, teow::GALAPAGOS_SCRUBLAND_MOSAIC),
        ],
    },
    SpeciesSeed {
        taxon_id: 11,
        ml_vision_id: Some(10),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Dendrocnide moroides",
        common_name_ru: "Гимпи-гимпи",
        synonyms: "жалящее дерево, гимпи",
        safety_status: "Toxic",
        habitat: HabitatType::Terrestrial,
        description_md: DENDROCNIDE_DESC,
        region_months: ALL_YEAR,
        regions: &[
            (Tdwg, tdwg::QLD),
            (Tdwg, tdwg::NSW),
            (Teow, teow::QUEENSLAND_TROPICAL_RAIN_FORESTS),
        ],
    },
    SpeciesSeed {
        taxon_id: 12,
        ml_vision_id: Some(11),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Adansonia digitata",
        common_name_ru: "Баобаб африканский",
        synonyms: "баобаб, обезьянье хлебное дерево, обезьяний хлеб",
        safety_status: "Edible",
        habitat: HabitatType::Terrestrial,
        description_md: ADANSONIA_DESC,
        region_months: ALL_YEAR,
        regions: &[
            (Tdwg, tdwg::KEN),
            (Tdwg, tdwg::TVL),
            (Teow, teow::NORTHERN_ACACIA_COMMIPHORA_BUSHLANDS),
            (Teow, teow::ZAMBEZIAN_AND_MOPANE_WOODLANDS),
        ],
    },
    SpeciesSeed {
        taxon_id: 13,
        ml_vision_id: Some(12),
        ml_audio_id: None,
        kingdom: "Plantae",
        scientific_name: "Sequoiadendron giganteum",
        common_name_ru: "Секвойядендрон гигантский",
        synonyms: "мамонтово дерево, гигантская секвойя, веллингтония",
        safety_status: "Protected",
        habitat: HabitatType::Terrestrial,
        description_md: SEQUOIADENDRON_DESC,
        region_months: ALL_YEAR,
        regions: &[(Tdwg, tdwg::CAL), (Teow, teow::SIERRA_NEVADA_FORESTS)],
    },
];

/// Exact-cell presence records (Stage 2A demo): `(lat, lon, taxon_id, months)`.
const SUMMER_AUTUMN: u16 = months(6, 10);
const BIRD_SEASON: u16 = months(5, 8);
const SPB: (f64, f64) = (59.93, 30.33);
const TOMSK: (f64, f64) = (56.48, 84.94);
pub const EXACT_PRESENCE: &[((f64, f64), i64, u16)] = &[
    (SPB, 1, SUMMER_AUTUMN),
    (SPB, 2, SUMMER_AUTUMN),
    (SPB, 3, SUMMER_AUTUMN),
    (SPB, 4, SUMMER_AUTUMN),
    (SPB, 5, BIRD_SEASON),
    (TOMSK, 2, SUMMER_AUTUMN),
    (TOMSK, 3, SUMMER_AUTUMN),
    (TOMSK, 5, BIRD_SEASON),
];

/// A key location whose H3 disk (resolution 4, `ring` cells around the center) receives
/// default ecoregions in `h3_geography`.
#[derive(Debug, Clone, Copy)]
pub struct GeoAnchor {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub ring: u32,
    pub regions: CellEcoregions,
}

const fn anchor(
    name: &'static str,
    (lat, lon): (f64, f64),
    ring: u32,
    tdwg_code: i64,
    teow_biome: i64,
    feow_basin: i64,
    meow_zone: i64,
) -> GeoAnchor {
    GeoAnchor {
        name,
        lat,
        lon,
        ring,
        regions: CellEcoregions {
            tdwg_code,
            teow_biome,
            feow_basin,
            meow_zone,
        },
    }
}

/// Key cells of the world. Overlapping disks merge field by field: a non-zero region
/// from a later anchor wins, zeros never erase (coastal anchors only add a MEOW zone).
pub const WORLD_ANCHORS: &[GeoAnchor] = &[
    // Europe & European Russia
    anchor("Saint Petersburg", SPB, 2, tdwg::RUW, teow::SARMATIC_MIXED_FORESTS, feow::NORTHERN_BALTIC_DRAINAGES, meow::BALTIC_SEA),
    anchor("Moscow", (55.75, 37.62), 2, tdwg::RUC, teow::SARMATIC_MIXED_FORESTS, feow::VOLGA_URAL, 0),
    anchor("Kandalaksha Bay, White Sea", (66.55, 33.10), 1, tdwg::RUN, teow::SCANDINAVIAN_AND_RUSSIAN_TAIGA, 0, meow::WHITE_SEA),
    anchor("Wadden Sea, Sylt", (54.90, 8.35), 1, tdwg::GER, teow::ATLANTIC_MIXED_FORESTS, feow::CENTRAL_AND_WESTERN_EUROPE, meow::NORTH_SEA),
    // Siberia
    anchor("Tomsk", TOMSK, 2, tdwg::WSB, teow::WEST_SIBERIAN_TAIGA, feow::UPPER_OB, 0),
    // North America
    anchor("Sequoia National Park", (36.56, -118.77), 2, tdwg::CAL, teow::SIERRA_NEVADA_FORESTS, feow::SACRAMENTO_SAN_JOAQUIN, 0),
    anchor("Tomales Bay", (38.15, -122.90), 1, tdwg::CAL, teow::NORTHERN_CALIFORNIA_COASTAL_FORESTS, 0, meow::NORTHERN_CALIFORNIA),
    anchor("Everglades, Florida Bay", (25.14, -80.92), 2, tdwg::FLA, teow::EVERGLADES, feow::FLORIDA_PENINSULA, meow::FLORIDIAN),
    // Caribbean & South America
    anchor("Guánica, Puerto Rico", (17.97, -66.88), 1, tdwg::PUE, teow::PUERTO_RICAN_DRY_FORESTS, 0, meow::GREATER_ANTILLES),
    anchor("Santa Cruz, Galápagos", (-0.74, -90.31), 1, tdwg::GAL, teow::GALAPAGOS_SCRUBLAND_MOSAIC, 0, meow::EASTERN_GALAPAGOS_ISLANDS),
    // Africa
    anchor("Tsavo East, Kenya", (-2.98, 38.47), 2, tdwg::KEN, teow::NORTHERN_ACACIA_COMMIPHORA_BUSHLANDS, 0, 0),
    anchor("Musina baobab belt, Limpopo", (-22.35, 30.04), 2, tdwg::TVL, teow::ZAMBEZIAN_AND_MOPANE_WOODLANDS, 0, 0),
    // Australia
    anchor("Daintree, Queensland", (-16.17, 145.42), 2, tdwg::QLD, teow::QUEENSLAND_TROPICAL_RAIN_FORESTS, 0, 0),
];

/// Row counts written by [`seed_catalog_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeedStats {
    pub taxa: usize,
    pub presence_cells: usize,
    pub region_bindings: usize,
    pub geography_cells: usize,
}

/// Zstd level for the in-memory demo (fast); `build-atlas` uses a higher level.
pub const DEMO_ZSTD_LEVEL: i32 = 3;

fn db_err(e: rusqlite::Error) -> AtlasError {
    AtlasError::DbError(e.to_string())
}

/// Seeds the full catalog with the demo compression level.
pub fn seed_catalog(conn: &Connection) -> Result<SeedStats, AtlasError> {
    seed_catalog_with(conn, DEMO_ZSTD_LEVEL)
}

/// Writes taxa (+FTS index), exact-cell presence, region bindings and world geography
/// into a database prepared by [`crate::db::init_schema`].
pub fn seed_catalog_with(conn: &Connection, zstd_level: i32) -> Result<SeedStats, AtlasError> {
    let mut stats = SeedStats::default();
    {
        let mut taxa = conn
            .prepare_cached(
                "INSERT OR REPLACE INTO taxa
                 (taxon_id, ml_vision_id, ml_audio_id, kingdom, scientific_name, common_name_ru,
                  synonyms, safety_status, habitat_type, description_zstd)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(db_err)?;
        let mut fts = conn
            .prepare_cached(
                "INSERT OR REPLACE INTO taxa_fts (rowid, scientific_name, common_name_ru, synonyms)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(db_err)?;
        let mut bind = conn
            .prepare_cached(
                "INSERT OR REPLACE INTO taxon_regions (region_scheme, region_code, taxon_id, months_bitmask)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .map_err(db_err)?;

        for s in CATALOG {
            let compressed = zstd::encode_all(s.description_md.as_bytes(), zstd_level)
                .map_err(|e| AtlasError::DbError(e.to_string()))?;
            taxa.execute(params![
                s.taxon_id,
                s.ml_vision_id,
                s.ml_audio_id,
                s.kingdom,
                s.scientific_name,
                s.common_name_ru,
                s.synonyms,
                s.safety_status,
                s.habitat.code(),
                compressed
            ])
            .map_err(db_err)?;
            fts.execute(params![s.taxon_id, s.scientific_name, s.common_name_ru, s.synonyms])
                .map_err(db_err)?;
            for &(scheme, code) in s.regions {
                bind.execute(params![scheme.code(), code, s.taxon_id, i64::from(s.region_months)])
                    .map_err(db_err)?;
                stats.region_bindings += 1;
            }
            stats.taxa += 1;
        }
    }

    {
        let mut presence = conn
            .prepare_cached(
                "INSERT OR REPLACE INTO geo_presence (h3_index, taxon_id, months_bitmask)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(db_err)?;
        for &((lat, lon), taxon_id, mask) in EXACT_PRESENCE {
            let h3 = geo::lat_lon_to_h3_res4(lat, lon)
                .ok_or_else(|| AtlasError::DbError(format!("invalid coordinates {lat}, {lon}")))?;
            presence
                .execute(params![h3, taxon_id, i64::from(mask)])
                .map_err(db_err)?;
            stats.presence_cells += 1;
        }
    }

    stats.geography_cells = seed_world_geography(conn)?;
    Ok(stats)
}

/// H3 resolution-4 cells within `ring` steps of `(lat, lon)`, as stored `i64` indexes
/// (same encoding as [`geo::lat_lon_to_h3_res4`]).
pub fn anchor_cells(lat: f64, lon: f64, ring: u32) -> Result<Vec<i64>, AtlasError> {
    let center = LatLng::new(lat, lon)
        .map_err(|e| AtlasError::DbError(format!("invalid anchor {lat}, {lon}: {e}")))?
        .to_cell(Resolution::Four);
    Ok(center
        .grid_disk::<Vec<_>>(ring)
        .into_iter()
        .map(|cell| u64::from(cell) as i64)
        .collect())
}

/// Fills `h3_geography` from [`WORLD_ANCHORS`]; returns the number of rows in the table.
pub fn seed_world_geography(conn: &Connection) -> Result<usize, AtlasError> {
    let mut upsert = conn
        .prepare_cached(
            "INSERT INTO h3_geography (h3_index, tdwg_code, teow_biome, feow_basin, meow_zone)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(h3_index) DO UPDATE SET
                 tdwg_code  = CASE WHEN excluded.tdwg_code  != 0 THEN excluded.tdwg_code  ELSE tdwg_code  END,
                 teow_biome = CASE WHEN excluded.teow_biome != 0 THEN excluded.teow_biome ELSE teow_biome END,
                 feow_basin = CASE WHEN excluded.feow_basin != 0 THEN excluded.feow_basin ELSE feow_basin END,
                 meow_zone  = CASE WHEN excluded.meow_zone  != 0 THEN excluded.meow_zone  ELSE meow_zone  END",
        )
        .map_err(db_err)?;
    for a in WORLD_ANCHORS {
        let r = a.regions;
        for cell in anchor_cells(a.lat, a.lon, a.ring)? {
            upsert
                .execute(params![cell, r.tdwg_code, r.teow_biome, r.feow_basin, r.meow_zone])
                .map_err(db_err)?;
        }
    }
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM h3_geography", [], |row| row.get(0))
        .map_err(db_err)?;
    Ok(rows as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn registered(scheme: RegionScheme, id: i64) -> bool {
        REGIONS.iter().any(|r| r.scheme == scheme && r.id == id)
    }

    #[test]
    fn region_registry_is_consistent() {
        let mut seen = HashSet::new();
        for r in REGIONS {
            assert!((1..=r.scheme.max_id()).contains(&r.id), "{r:?} out of range");
            assert!(seen.insert((r.scheme, r.id)), "duplicate {r:?}");
            assert_eq!(r.scheme == Tdwg, r.code.len() == 3, "{r:?}");
        }
        for s in CATALOG {
            for &(scheme, id) in s.regions {
                assert!(registered(scheme, id), "{} → unregistered {scheme:?} {id}", s.scientific_name);
            }
        }
        for a in WORLD_ANCHORS {
            for scheme in RegionScheme::ALL {
                if let Some(id) = a.regions.region(scheme) {
                    assert!(registered(scheme, id), "{} → unregistered {scheme:?} {id}", a.name);
                }
            }
        }
    }

    #[test]
    fn catalog_ids_are_unique_and_habitats_match_ecology() {
        let taxa: HashSet<i64> = CATALOG.iter().map(|s| s.taxon_id).collect();
        let vision: HashSet<i64> = CATALOG.iter().filter_map(|s| s.ml_vision_id).collect();
        assert_eq!(taxa.len(), CATALOG.len());
        assert_eq!(vision.len(), CATALOG.len());

        let habitat = |name: &str| CATALOG.iter().find(|s| s.scientific_name == name).unwrap().habitat;
        assert_eq!(habitat("Cicuta virosa"), HabitatType::Wetland);
        assert_eq!(habitat("Nuphar lutea"), HabitatType::Freshwater);
        assert_eq!(habitat("Typha latifolia"), HabitatType::Wetland);
        assert_eq!(habitat("Zostera marina"), HabitatType::Marine);
        assert_eq!(habitat("Sequoiadendron giganteum"), HabitatType::Terrestrial);

        // Aquatic taxa need a freshwater or marine range, not only botanical countries.
        for s in CATALOG.iter().filter(|s| s.habitat != HabitatType::Terrestrial) {
            let wanted = if s.habitat == HabitatType::Marine { Meow } else { Feow };
            assert!(s.regions.iter().any(|&(sc, _)| sc == wanted), "{}", s.scientific_name);
        }
    }

    #[test]
    fn seeds_fresh_database() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        let stats = seed_catalog(&conn).unwrap();
        assert_eq!(stats.taxa, CATALOG.len());
        assert_eq!(stats.presence_cells, EXACT_PRESENCE.len());
        assert_eq!(
            stats.region_bindings,
            CATALOG.iter().map(|s| s.regions.len()).sum::<usize>()
        );
        assert!(stats.geography_cells >= WORLD_ANCHORS.len() * 7);

        // Every anchor center resolves, and coastal merges keep the land regions.
        for a in WORLD_ANCHORS {
            let center = geo::lat_lon_to_h3_res4(a.lat, a.lon).unwrap();
            let (tdwg, meow): (i64, i64) = conn
                .query_row(
                    "SELECT tdwg_code, meow_zone FROM h3_geography WHERE h3_index = ?1",
                    params![center],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(tdwg, a.regions.tdwg_code, "{}", a.name);
            assert_eq!(meow, a.regions.meow_zone, "{}", a.name);
        }

        let blob: Vec<u8> = conn
            .query_row("SELECT description_zstd FROM taxa WHERE taxon_id = 6", [], |r| r.get(0))
            .unwrap();
        let text = String::from_utf8(zstd::decode_all(&blob[..]).unwrap()).unwrap();
        assert!(text.contains("цикутотоксин"));
    }
}
