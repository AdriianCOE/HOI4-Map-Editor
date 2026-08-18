//! Anything relating to loading or saving map data
use ahash::{AHashMap, AHashSet};
use defy::Contextualize;
use image::codecs::bmp::{BmpDecoder, BmpEncoder};
use image::{ColorType, DynamicImage, Pixel, Rgb, RgbImage, Rgba, RgbaImage};
use uord::UOrd2 as UOrd;

use super::{
    Bundle, Color, ConnectionData, Map, MapBase, ProvinceData, ProvinceIdIndex, random_color_pure,
};
use crate::app::format::{Adjacency, Definition, ParseCsv};
use crate::config::Config;
use crate::error::Error;
use crate::util::files::Location;

use std::collections::hash_map::Entry;
use std::io::{self, Cursor, Read, Write};
use std::sync::Arc;

pub(super) fn load_bundle(location: &Location, config: Config) -> Result<Bundle, Error> {
    let (province_image, definition_table, adjacencies_table, rivers) =
        location.clone().manipulate_files(|files| {
            let province_image = read_rgb_bmp_image(files.open_file("provinces.bmp")?)?;
            let definition_table = read_definition_table(files.open_file("definition.csv")?)?;
            let adjacencies_table = files
                .open_file_maybe_not_found("adjacencies.csv")?
                .map_or_else(|| Ok(Vec::new()), read_adjacencies_table)?;
            let rivers = files
                .open_file_maybe_not_found("rivers.bmp")?
                .map(read_rgb_bmp_image)
                .transpose()?;
            Ok((province_image, definition_table, adjacencies_table, rivers))
        })?;

    construct_map_data(
        province_image,
        definition_table,
        adjacencies_table,
        rivers,
        config,
    )
}

pub(super) fn construct_map_data(
    province_image: RgbImage,
    definition_table: Vec<Definition>,
    adjacencies_table: Vec<Adjacency>,
    rivers: Option<RgbImage>,
    config: Config,
) -> Result<Bundle, Error> {
    construct_map_data_inner(
        province_image,
        definition_table,
        adjacencies_table,
        rivers,
        config,
    )
}

#[cfg(test)]
pub(crate) fn construct_map_data_for_sparse_tests(
    province_image: RgbImage,
    definition_table: Vec<Definition>,
    adjacencies_table: Vec<Adjacency>,
    rivers: Option<RgbImage>,
    config: Config,
) -> Result<Bundle, Error> {
    construct_map_data(
        province_image,
        definition_table,
        adjacencies_table,
        rivers,
        config,
    )
}

fn construct_map_data_inner(
    province_image: RgbImage,
    definition_table: Vec<Definition>,
    adjacencies_table: Vec<Adjacency>,
    rivers: Option<RgbImage>,
    config: Config,
) -> Result<Bundle, Error> {
    let mut color_buffer = province_image;

    let preserved_id_count = u32::try_from(definition_table.len())
        .map_err(|_| Error::from("definition.csv contains too many province records"))?;
    if preserved_id_count == 0 {
        return Err("definition.csv contains no province records".into());
    };

    let definition_id_index = ProvinceIdIndex::from_pairs(
        definition_table
            .iter()
            .map(|definition| (definition.id, definition.rgb)),
    )
    .map_err(|error| Error::from(format!("definition.csv {error}")))?;

    // Initially convert the definition table into a province data map
    let mut definition_map = definition_table
        .into_iter()
        .map(|d| (d.rgb, ProvinceData::from_definition_config(d, &config)))
        .collect::<AHashMap<Color, ProvinceData>>();
    // Loop through every pixel in the color buffer, ensuring that the resulting province data map
    // will be valid and will have no provinces mapping to colors not on the color buffer
    let mut province_data_map = AHashMap::default();
    for (x, y, &Rgb(pixel)) in color_buffer.enumerate_pixels() {
        // If this color isn't in the new province data map, but it is in the definition table,
        // take it from the former and put it in the latter
        match province_data_map.entry(pixel) {
            Entry::Vacant(entry) => {
                let mut province_data = definition_map.remove(&pixel).unwrap_or_default();
                province_data.add_pixel([x, y]);
                entry.insert(Arc::new(province_data));
            }
            Entry::Occupied(entry) => {
                let entry = Arc::make_mut(entry.into_mut());
                entry.add_pixel([x, y]);
            }
        };
    }

    province_data_map.shrink_to_fit();
    let _ = definition_map;

    let loaded_id_index = ProvinceIdIndex::from_pairs(
        definition_id_index
            .iter()
            .filter(|(_, color)| province_data_map.contains_key(color)),
    )
    .expect("definition index cannot become ambiguous after filtering loaded colors");

    // Adjacency endpoints are external identities. Resolve them by keyed lookup;
    // malformed or unsupported records are preserved for validation and round-trip.
    let mut preserved_unsupported_adjacencies = Vec::new();
    let mut connection_data_map = AHashMap::with_capacity(adjacencies_table.len());
    for a in adjacencies_table.into_iter() {
        let resolve_color = |id| loaded_id_index.color_for_id(id);
        if let Some(rel) = UOrd::new([a.from_id, a.to_id]).try_map_opt(resolve_color) {
            if let Some(connection_data) = ConnectionData::from_adjacency(a.clone(), resolve_color)
            {
                connection_data_map.insert(rel, Arc::new(connection_data));
            } else {
                preserved_unsupported_adjacencies.push(a);
            };
        } else {
            preserved_unsupported_adjacencies.push(a);
        };
    }

    connection_data_map.shrink_to_fit();

    // Recolor the entire map if `preserve_ids` is false
    if !config.preserve_ids {
        recolor_everything(
            &mut color_buffer,
            &mut province_data_map,
            &mut connection_data_map,
        );
    };

    let rivers_overlay = rivers.as_ref().map(process_and_clear_rivers_image);
    let province_id_index = ProvinceIdIndex::from_pairs(
        province_data_map
            .iter()
            .filter_map(|(&color, province)| province.preserved_id.map(|id| (id, color))),
    )
    .expect("loader has already validated preserved province identities");

    let mut map = Map {
        base: MapBase {
            color_buffer: Arc::new(color_buffer),
            province_data_map: Arc::new(province_data_map),
            province_id_index: Arc::new(province_id_index),
            connection_data_map: Arc::new(connection_data_map),
            rivers_overlay: rivers_overlay.map(Arc::new),
        },
        boundaries: AHashMap::default(),
        preserved_unsupported_adjacencies,
    };

    map.recalculate_all_boundaries();

    Ok(Bundle { map, config })
}

pub(super) fn recolor_everything(
    color_buffer: &mut RgbImage,
    province_data_map: &mut AHashMap<Color, Arc<ProvinceData>>,
    connection_data_map: &mut AHashMap<UOrd<Color>, Arc<ConnectionData>>,
) {
    let mut colors_list = AHashSet::with_capacity(province_data_map.len());
    let mut replacement_map = AHashMap::with_capacity(province_data_map.len());

    let mut new_province_data_map = AHashMap::with_capacity(province_data_map.len());
    for (previous_color, province_data) in province_data_map.drain() {
        let color = random_color_pure(&colors_list, province_data.kind);
        let opt = colors_list.insert(color);
        debug_assert!(opt);
        let opt = replacement_map.insert(previous_color, color);
        debug_assert_eq!(opt, None);
        let opt = new_province_data_map.insert(color, province_data);
        debug_assert_eq!(opt, None);
    }

    *province_data_map = new_province_data_map;

    let mut new_connection_data_map = AHashMap::with_capacity(connection_data_map.len());
    for (previous_rel, mut connection_data) in connection_data_map.drain() {
        // Replace `through`'s color with the new one
        let connection_data_mut = Arc::make_mut(&mut connection_data);
        connection_data_mut.through = connection_data_mut
            .through
            .and_then(|t| replacement_map.get(&t).copied());
        if let Some(rel) = previous_rel.try_map_opt(|color| replacement_map.get(&color).copied()) {
            new_connection_data_map.insert(rel, connection_data);
        };
    }

    *connection_data_map = new_connection_data_map;

    for Rgb(pixel) in color_buffer.pixels_mut() {
        *pixel = replacement_map[pixel];
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum IdChange {
    AssignedNew(u32),
}

impl ToString for IdChange {
    fn to_string(&self) -> String {
        match self {
            IdChange::AssignedNew(id) => format!("Assigned ID {} to new province", id),
        }
    }
}

pub(super) type MapData = (Vec<Definition>, Vec<Adjacency>, Option<Vec<IdChange>>);

#[derive(Debug, Clone, PartialEq)]
pub struct SaveOperation {
    pub had_id_changes: bool,
}

pub fn save_bundle(location: &Location, bundle: &Bundle) -> Result<SaveOperation, Error> {
    super::province_save::save_bundle_compat(location, bundle)
}

pub(super) fn deconstruct_map_data(bundle: &Bundle) -> Result<MapData, Error> {
    if bundle.config.preserve_ids {
        deconstruct_map_data_preserve_ids(bundle)
    } else {
        deconstruct_map_data_no_preserve_ids(bundle)
    }
}

fn deconstruct_map_data_preserve_ids(bundle: &Bundle) -> Result<MapData, Error> {
    let map = &bundle.map;
    let mut definitions_table = Vec::with_capacity(map.provinces_count());
    let mut color_index = AHashMap::with_capacity(map.provinces_count());

    // Existing external identities come from the ordered index, never from a
    // row position or hash-map iteration. This keeps sparse IDs intact.
    for (id, color) in map.province_id_index().iter() {
        let province = map.base.province_data_map.get(&color).ok_or_else(|| {
            Error::from(format!(
                "province ID {id} refers to a color missing from the map"
            ))
        })?;
        if province.preserved_id != Some(id) {
            return Err(format!(
                "province ID index and province data disagree for color {color:?}"
            )
            .into());
        }
        let definition = province.to_definition_with_id(color, id)?;
        color_index.insert(color, id);
        definitions_table.push(definition);
    }

    // Newly painted colors have no external identity yet. Color order is stable
    // across hash seeds, so multiple allocations are deterministic.
    let mut new_colors = map
        .base
        .province_data_map
        .iter()
        .filter_map(|(&color, province)| {
            map.province_id_index()
                .id_for_color(color)
                .is_none()
                .then_some((color, province))
        })
        .collect::<Vec<_>>();
    new_colors.sort_unstable_by_key(|&(color, _)| color);

    let mut changes = Vec::new();
    let mut next_id = map
        .province_id_index()
        .next_allocatable_id()
        .map_err(|error| Error::from(error.to_string()))?;
    let new_count = new_colors.len();
    for (position, (color, province)) in new_colors.into_iter().enumerate() {
        if province.preserved_id.is_some() {
            return Err(format!(
                "province color {color:?} has a preserved ID but is missing from the ID index"
            )
            .into());
        }
        let definition = province.to_definition_with_id(color, next_id)?;
        color_index.insert(color, next_id);
        definitions_table.push(definition);
        changes.push(IdChange::AssignedNew(next_id));
        if position + 1 < new_count {
            next_id = next_id
                .checked_add(1)
                .ok_or_else(|| Error::from("no province ID is available after u32::MAX"))?;
        }
    }

    definitions_table.sort_by_key(|definition| definition.id);

    let mut adjacencies_table = Vec::with_capacity(bundle.map.connections_count());
    for (&rel, connection_data) in bundle.map.base.connection_data_map.iter() {
        let rel = rel.map(|color| color_index[&color]);
        adjacencies_table.push(connection_data.to_adjacency(rel, |t| color_index[&t]));
    }

    adjacencies_table.extend_from_slice(&bundle.map.preserved_unsupported_adjacencies);
    adjacencies_table.sort();

    let id_changes = if changes.is_empty() {
        None
    } else {
        Some(changes)
    };
    Ok((definitions_table, adjacencies_table, id_changes))
}

fn deconstruct_map_data_no_preserve_ids(bundle: &Bundle) -> Result<MapData, Error> {
    let mut definitions_table = Vec::with_capacity(bundle.map.provinces_count());
    for (&color, province_data) in bundle.map.base.province_data_map.iter() {
        definitions_table.push(province_data.to_definition_with_id(color, 0)?);
    }

    definitions_table.sort();

    let mut id = 1;
    let mut color_index = AHashMap::with_capacity(definitions_table.len());
    for definition in definitions_table.iter_mut() {
        color_index.insert(definition.rgb, id);
        definition.id = id;
        id += 1;
    }

    let mut adjacencies_table = Vec::with_capacity(bundle.map.connections_count());
    for (&rel, connection_data) in bundle.map.base.connection_data_map.iter() {
        let rel = rel.map(|color| color_index[&color]);
        adjacencies_table.push(connection_data.to_adjacency(rel, |t| color_index[&t]));
    }

    adjacencies_table.sort();

    Ok((definitions_table, adjacencies_table, None))
}

fn process_and_clear_rivers_image(img: &RgbImage) -> RgbaImage {
    //const RIVERS_PIXEL_PALETTE_CLEAR: &[Rgb<u8>] = &[
    //  // land
    //  Rgb([255, 255, 255]),
    //  // water
    //  Rgb([122, 122, 122])
    //];

    const RIVERS_PIXEL_PALETTE_KEEP: &[Rgb<u8>] = &[
        // river source
        Rgb([0, 255, 0]),
        // flow-in source
        Rgb([255, 0, 0]),
        // flow-out source
        Rgb([255, 252, 0]),
        // rivers
        Rgb([0, 225, 255]),
        Rgb([0, 200, 255]),
        Rgb([0, 150, 255]),
        Rgb([0, 100, 255]),
        Rgb([0, 0, 255]),
        Rgb([0, 0, 225]),
        Rgb([0, 0, 200]),
        Rgb([0, 0, 150]),
        Rgb([0, 0, 100]),
    ];

    RgbaImage::from_par_fn(img.width(), img.height(), |x, y| {
        let pixel = img.get_pixel(x, y);
        if RIVERS_PIXEL_PALETTE_KEEP.contains(pixel) {
            pixel.to_rgba()
        } else {
            Rgba([0x00; 4])
        }
    })
}

pub fn read_rgb_bmp_image<R: Read>(reader: R) -> Result<RgbImage, Error> {
    let decoder = BmpDecoder::new(read_all(reader).context("failed to read bmp image")?)?;
    let img = DynamicImage::from_decoder(decoder)?;
    Ok(img.into_rgb8())
}

fn read_definition_table<R: Read>(reader: R) -> Result<Vec<Definition>, Error> {
    Definition::read_records(reader).map_err(|err| Error::Csv(err, "definition.csv"))
}

fn read_adjacencies_table<R: Read>(reader: R) -> Result<Vec<Adjacency>, Error> {
    Adjacency::read_records(reader).map_err(|err| Error::Csv(err, "adjacencies.csv"))
}

pub fn write_rgb_bmp_image<W: Write>(
    mut writer: W,
    province_image: &RgbImage,
) -> Result<(), Error> {
    let mut encoder = BmpEncoder::new(&mut writer);
    let (width, height) = province_image.dimensions();
    encoder
        .encode(province_image.as_raw(), width, height, ColorType::Rgb8)
        .map_err(From::from)
}

pub(super) fn write_definition_table<W: Write>(
    writer: W,
    definition_table: Vec<Definition>,
) -> Result<(), Error> {
    Definition::write_records(&definition_table, writer)
        .map_err(|err| Error::Csv(err, "definition.csv"))
}

pub(super) fn write_adjacencies_table<W: Write>(
    writer: W,
    adjacencies_table: Vec<Adjacency>,
) -> Result<(), Error> {
    Adjacency::write_records(&adjacencies_table, writer)
        .map_err(|err| Error::Csv(err, "adjacencies.csv"))
}

pub(super) fn write_id_changes<W: Write>(
    mut writer: W,
    id_changes: Vec<IdChange>,
) -> Result<(), Error> {
    writeln!(writer, "ID Changes").context("failed to write id changes to file")?;
    for id_change in id_changes {
        writeln!(writer, "- {}", id_change.to_string())
            .context("failed to write id changes to file")?;
    }

    Ok(())
}

fn read_all<R: Read>(mut reader: R) -> io::Result<Cursor<Vec<u8>>> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    Ok(Cursor::new(buf))
}

#[cfg(test)]
mod tests {
    use super::{construct_map_data, construct_map_data_for_sparse_tests};
    use crate::app::format::{Adjacency, AdjacencyKind, Definition, DefinitionKind, ParseCsv};
    use crate::app::map::{Bundle, History, Problem};
    use crate::config::Config;
    use crate::util::files::Location;
    use image::{Rgb, RgbImage};
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn empty_definition_table_returns_error_instead_of_panicking() {
        let result = construct_map_data(
            RgbImage::new(1, 1),
            Vec::new(),
            Vec::new(),
            None,
            Config::default(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn sparse_definition_ids_load_with_their_exact_external_identity() {
        let colors = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let image = RgbImage::from_fn(4, 1, |x, _| Rgb(colors[x as usize]));
        let definitions = colors
            .into_iter()
            .zip([1, 7, 42, 500])
            .map(|(rgb, id)| Definition {
                id,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();

        let bundle = construct_map_data(
            image,
            definitions,
            Vec::new(),
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap();
        assert_eq!(bundle.map.provinces_count(), 4);
        assert_eq!(bundle.map.province_id_index().max_id(), Some(500));
        assert_eq!(
            bundle.map.province_ids().collect::<Vec<_>>(),
            vec![1, 7, 42, 500]
        );
        assert!(!bundle.map.contains_province_id(2));
    }

    fn bundle_with_forced_ids(ids: &[u32]) -> (Bundle, Vec<[u8; 3]>) {
        let colors = ids
            .iter()
            .enumerate()
            .map(|(index, _)| [10 + index as u8 * 20, 20, 30])
            .collect::<Vec<_>>();
        let image = RgbImage::from_fn(colors.len() as u32, 1, |x, _| Rgb(colors[x as usize]));
        let definitions = colors
            .iter()
            .copied()
            .enumerate()
            .map(|(index, rgb)| Definition {
                id: index as u32 + 1,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();
        let mut bundle = construct_map_data(
            image,
            definitions,
            Vec::new(),
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap();
        for (&color, &id) in colors.iter().zip(ids) {
            let province_data = Arc::make_mut(
                Arc::make_mut(&mut bundle.map.base.province_data_map)
                    .get_mut(&color)
                    .unwrap(),
            );
            province_data.preserved_id = Some(id);
        }
        bundle.map.rebuild_province_id_index();
        (bundle, colors)
    }

    fn sparse_adjacency(from_id: u32, to_id: u32, through: Option<u32>) -> Adjacency {
        Adjacency {
            from_id,
            to_id,
            kind: AdjacencyKind::Sea,
            through,
            start: None,
            stop: None,
            rule_name: String::new(),
            comment: String::new(),
        }
    }

    fn sparse_bundle_with_adjacencies(ids: &[u32], adjacencies: Vec<Adjacency>) -> Bundle {
        let colors = ids
            .iter()
            .enumerate()
            .map(|(index, _)| [10 + index as u8 * 20, 20, 30])
            .collect::<Vec<_>>();
        let image = RgbImage::from_fn(colors.len() as u32, 1, |x, _| Rgb(colors[x as usize]));
        let definitions = colors
            .iter()
            .copied()
            .zip(ids.iter().copied())
            .map(|(rgb, id)| Definition {
                id,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();
        construct_map_data_for_sparse_tests(
            image,
            definitions,
            adjacencies,
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn sparse_adjacency_endpoints_resolve_by_external_id_without_dense_storage() {
        let bundle = sparse_bundle_with_adjacencies(
            &[1, 7, 42, 500],
            vec![
                sparse_adjacency(7, 500, Some(42)),
                sparse_adjacency(1, 42, None),
                sparse_adjacency(42, 500, None),
            ],
        );
        let mut endpoints = bundle
            .map
            .iter_connection_data()
            .map(|(relation, connection)| {
                let ids = relation.map(|color| bundle.map.province_id_for_color(color).unwrap());
                (
                    ids.into_array(),
                    connection
                        .through
                        .map(|color| bundle.map.province_id_for_color(color).unwrap()),
                )
            })
            .collect::<Vec<_>>();
        endpoints.sort_unstable();
        assert_eq!(
            endpoints,
            vec![([1, 42], None), ([7, 500], Some(42)), ([42, 500], None)]
        );
        assert!(bundle.map.adjacency_references_province_id(7));
        assert!(bundle.map.adjacency_references_province_id(500));
        assert!(bundle.map.adjacency_references_province_id(42));
        assert!(bundle.map.unresolved_adjacencies().is_empty());

        let (_, adjacencies, _) = super::deconstruct_map_data(&bundle).unwrap();
        let mut serialized_endpoints = adjacencies
            .iter()
            .map(|adjacency| (adjacency.from_id, adjacency.to_id, adjacency.through))
            .collect::<Vec<_>>();
        serialized_endpoints.sort_unstable();
        assert_eq!(
            serialized_endpoints,
            vec![(1, 42, None), (7, 500, Some(42)), (42, 500, None)]
        );
    }

    #[test]
    fn high_and_missing_sparse_adjacency_endpoints_are_keyed_and_preserved() {
        let bundle = sparse_bundle_with_adjacencies(
            &[1, 10_000],
            vec![
                sparse_adjacency(1, 10_000, None),
                sparse_adjacency(1, 999, None),
            ],
        );
        assert_eq!(bundle.map.connections_count(), 1);
        assert_eq!(bundle.map.unresolved_adjacencies().len(), 1);
        assert!(bundle.map.adjacency_references_province_id(999));
        assert_eq!(bundle.map.province_id_index().province_count(), 2);

        let (_, adjacencies, _) = super::deconstruct_map_data(&bundle).unwrap();
        let mut serialized_endpoints = adjacencies
            .iter()
            .map(|adjacency| (adjacency.from_id, adjacency.to_id, adjacency.through))
            .collect::<Vec<_>>();
        serialized_endpoints.sort_unstable();
        assert_eq!(
            serialized_endpoints,
            vec![(1, 999, None), (1, 10_000, None)]
        );
    }

    #[test]
    fn province_history_paints_pixels_and_terrain_with_undo_redo() {
        let first = [10, 20, 30];
        let second = [40, 50, 60];
        let image = RgbImage::from_fn(3, 1, |x, _| Rgb(if x < 2 { first } else { second }));
        let definitions = [first, second]
            .into_iter()
            .enumerate()
            .map(|(index, rgb)| Definition {
                id: index as u32 + 1,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };
        let mut bundle = construct_map_data(image, definitions, Vec::new(), None, config).unwrap();
        assert_eq!(
            bundle.map.province_id_index().iter().collect::<Vec<_>>(),
            vec![(1, first), (2, second)]
        );
        assert!(bundle.map.province_id_index().is_contiguous_from_one());
        let mut history = History::new(8, &bundle.map);

        assert!(
            history
                .paint_province_terrain(&mut bundle, [0, 0], "forest".to_owned())
                .is_some()
        );
        assert_eq!(bundle.map.get_province(first).terrain, "forest");
        assert!(history.undo(&mut bundle.map).is_some());
        assert_eq!(bundle.map.get_province(first).terrain, "plains");
        assert!(history.redo(&mut bundle.map).is_some());
        assert_eq!(bundle.map.get_province(first).terrain, "forest");

        assert!(
            history
                .paint_pixel(&mut bundle, [1, 0], second, 1)
                .is_some()
        );
        assert_eq!(bundle.map.get_color_at([1, 0]), second);
        assert!(history.undo(&mut bundle.map).is_some());
        assert_eq!(bundle.map.get_color_at([1, 0]), first);
        assert!(history.redo(&mut bundle.map).is_some());

        let root = std::env::temp_dir().join(format!("hoi4-province-edit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let location = Location::Directory(root.clone());
        bundle.save(&location).unwrap();
        let reloaded = Bundle::load(&location, bundle.config.clone()).unwrap();
        assert_eq!(reloaded.map.get_color_at([1, 0]), second);
        assert_eq!(reloaded.map.get_province(first).terrain, "forest");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_map_dimensions_preserve_texture_boundaries_and_diagnostics() {
        let land = [10, 20, 30];
        let ocean = [40, 50, 60];
        let definitions = vec![
            Definition {
                id: 1,
                rgb: land,
                kind: DefinitionKind::Land,
                coastal: true,
                terrain: "plains".to_owned(),
                continent: 1,
            },
            Definition {
                id: 2,
                rgb: ocean,
                kind: DefinitionKind::Sea,
                coastal: false,
                terrain: "ocean".to_owned(),
                continent: 0,
            },
        ];
        let config = Config {
            preserve_ids: true,
            ..Config::default()
        };

        for [width, height] in [
            [3, 2],
            [64, 64],
            [128, 128],
            [256, 128],
            [192, 64],
            [130, 70],
        ] {
            let mut image = RgbImage::from_pixel(width, height, Rgb(ocean));
            image.put_pixel(0, 0, Rgb(land));
            let bundle =
                construct_map_data(image, definitions.clone(), Vec::new(), None, config.clone())
                    .unwrap();

            assert_eq!(bundle.map.dimensions(), [width, height]);
            assert_eq!(
                bundle.map.gen_texture_buffer(|color| color).dimensions(),
                (width, height)
            );
            assert!(bundle.map.iter_boundaries().next().is_some());
            assert!(
                !bundle
                    .generate_problems()
                    .iter()
                    .any(|problem| matches!(problem, Problem::TooLargeBox(_))),
                "small or ocean-heavy {width}x{height} map produced a false large-box warning"
            );
        }

        let image = RgbImage::from_pixel(130, 70, Rgb(ocean));
        let bundle = construct_map_data(image, definitions, Vec::new(), None, config).unwrap();
        let problems = bundle.generate_problems();
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, Problem::InvalidWidth))
        );
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, Problem::InvalidHeight))
        );
    }

    #[test]
    fn preserve_id_deconstruction_keeps_sparse_ids_through_csv_round_trip() {
        let (bundle, _) = bundle_with_forced_ids(&[1, 7, 42, 500]);
        let (definitions, _, changes) = super::deconstruct_map_data(&bundle).unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![1, 7, 42, 500]
        );
        assert_eq!(changes, None, "existing sparse IDs are not renumbered");

        let mut csv = Vec::new();
        super::write_definition_table(&mut csv, definitions).unwrap();
        let parsed = Definition::read_records(csv.as_slice()).unwrap();
        assert_eq!(
            parsed
                .iter()
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![1, 7, 42, 500]
        );
    }

    #[test]
    fn deletion_keeps_the_remaining_sparse_ids_and_gap() {
        let (mut bundle, colors) = bundle_with_forced_ids(&[1, 7, 42, 500]);
        bundle.map.merge_province_into(colors[2], colors[1]);

        let (definitions, _, changes) = super::deconstruct_map_data(&bundle).unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![1, 7, 500]
        );
        assert_eq!(changes, None);
    }

    #[test]
    fn newly_painted_provinces_keep_the_ids_assigned_during_editing() {
        let colors = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let image_colors = [
            colors[0], colors[1], colors[0], colors[2], colors[0], colors[3],
        ];
        let image = RgbImage::from_fn(6, 1, |x, _| Rgb(image_colors[x as usize]));
        let definitions = colors
            .into_iter()
            .enumerate()
            .map(|(index, rgb)| Definition {
                id: index as u32 + 1,
                rgb,
                kind: DefinitionKind::Land,
                coastal: false,
                terrain: "plains".to_owned(),
                continent: 1,
            })
            .collect();
        let mut bundle = construct_map_data(
            image,
            definitions,
            Vec::new(),
            None,
            Config {
                preserve_ids: true,
                ..Config::default()
            },
        )
        .unwrap();
        for (color, id) in colors.into_iter().zip([1, 7, 42, 500]) {
            Arc::make_mut(
                Arc::make_mut(&mut bundle.map.base.province_data_map)
                    .get_mut(&color)
                    .unwrap(),
            )
            .preserved_id = Some(id);
        }
        bundle.map.rebuild_province_id_index();
        bundle.map.merge_province_into(colors[1], colors[2]);

        let lower_color = [1, 2, 3];
        let higher_color = [250, 2, 3];
        bundle.map.flood_fill_province([0, 0], higher_color);
        bundle.map.flood_fill_province([2, 0], lower_color);
        for color in [lower_color, higher_color] {
            let province = bundle.map.get_province_mut(color);
            province.kind = super::super::ProvinceKind::Land;
            province.terrain = "plains".to_owned();
            province.continent = 1;
            province.coastal = Some(false);
        }

        let (definitions, _, changes) = super::deconstruct_map_data(&bundle).unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| (definition.id, definition.rgb))
                .collect::<Vec<_>>(),
            vec![
                (1, colors[0]),
                (42, colors[2]),
                (500, colors[3]),
                (501, higher_color),
                (502, lower_color),
            ]
        );
        assert_eq!(changes, None);
    }

    #[test]
    fn high_sparse_ids_do_not_create_gap_records_and_overflow_is_an_error() {
        let (bundle, _) = bundle_with_forced_ids(&[1, 10_000]);
        let (definitions, _, _) = super::deconstruct_map_data(&bundle).unwrap();
        assert_eq!(definitions.len(), 2);
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.id)
                .collect::<Vec<_>>(),
            vec![1, 10_000]
        );

        let (mut exhausted, colors) = bundle_with_forced_ids(&[1, u32::MAX]);
        exhausted.map.flood_fill_province([0, 0], [1, 2, 3]);
        let province = exhausted.map.get_province_mut([1, 2, 3]);
        province.kind = super::super::ProvinceKind::Land;
        province.terrain = "plains".to_owned();
        province.continent = 1;
        province.coastal = Some(false);
        assert!(
            super::deconstruct_map_data(&exhausted)
                .unwrap_err()
                .to_string()
                .contains("u32::MAX")
        );
        assert_eq!(colors.len(), 2);
    }
}
