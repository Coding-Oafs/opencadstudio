//! Scalable LAS/LAZ metadata, sampling, classification edits, and export.
//!
//! The crate deliberately keeps the source cloud outside the CAD document.  A
//! viewer can retain only a bounded display sample plus a sparse set of edits,
//! then stream the original file when it is time to export a revised LAS/LAZ.

use las::{point::Classification, Header, Point, Reader, Writer};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error, fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Errors returned by point-cloud operations.
#[derive(Debug)]
pub enum Error {
    Las(las::Error),
    Io(io::Error),
    InvalidLimit(&'static str),
    UnsupportedExtension(PathBuf),
    OutputExists(PathBuf),
    SameInputAndOutput(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Las(error) => write!(f, "LAS/LAZ error: {error}"),
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::InvalidLimit(name) => write!(f, "{name} must be greater than zero"),
            Self::UnsupportedExtension(path) => write!(
                f,
                "point-cloud output must have a .las or .laz extension: {}",
                path.display()
            ),
            Self::OutputExists(path) => {
                write!(
                    f,
                    "refusing to overwrite existing output: {}",
                    path.display()
                )
            }
            Self::SameInputAndOutput(path) => write!(
                f,
                "input and output point-cloud paths must differ: {}",
                path.display()
            ),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Las(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<las::Error> for Error {
    fn from(value: las::Error) -> Self {
        Self::Las(value)
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// File-level information that can be read without loading point records.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudMetadata {
    pub point_count: u64,
    pub version_major: u8,
    pub version_minor: u8,
    pub point_format: u8,
    pub compressed: bool,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub scales: [f64; 3],
    pub offsets: [f64; 3],
    pub system_identifier: String,
    pub generating_software: String,
    pub creation_date: Option<String>,
    pub file_source_id: u16,
    pub has_crs: bool,
    pub vlr_count: usize,
    pub evlr_count: usize,
}

impl CloudMetadata {
    fn from_header(header: &Header) -> Result<Self> {
        let version = header.version();
        let format = header.point_format();
        let bounds = header.bounds();
        let transforms = header.transforms();

        Ok(Self {
            point_count: header.number_of_points(),
            version_major: version.major,
            version_minor: version.minor,
            point_format: format.to_u8()?,
            compressed: format.is_compressed,
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z],
            scales: [transforms.x.scale, transforms.y.scale, transforms.z.scale],
            offsets: [
                transforms.x.offset,
                transforms.y.offset,
                transforms.z.offset,
            ],
            system_identifier: header.system_identifier().to_owned(),
            generating_software: header.generating_software().to_owned(),
            creation_date: header.date().map(|date| date.to_string()),
            file_source_id: header.file_source_id(),
            has_crs: header.has_crs_vlrs(),
            vlr_count: header.vlrs().len(),
            evlr_count: header.evlrs().len(),
        })
    }
}

/// A display-oriented point retaining the attributes needed for inspection and
/// classification workflows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SamplePoint {
    /// Zero-based position of the point in the source file.
    pub source_index: u64,
    pub position: [f64; 3],
    pub intensity: u16,
    pub classification: u8,
    pub return_number: u8,
    pub number_of_returns: u8,
    pub scan_angle: f32,
    pub user_data: u8,
    pub point_source_id: u16,
    pub gps_time: Option<f64>,
    pub color: Option<[u16; 3]>,
    pub nir: Option<u16>,
    pub is_synthetic: bool,
    pub is_key_point: bool,
    pub is_withheld: bool,
    pub is_overlap: bool,
}

impl SamplePoint {
    fn from_point(source_index: u64, point: Point) -> Self {
        Self {
            source_index,
            position: [point.x, point.y, point.z],
            intensity: point.intensity,
            classification: u8::from(point.classification),
            return_number: point.return_number,
            number_of_returns: point.number_of_returns,
            scan_angle: point.scan_angle,
            user_data: point.user_data,
            point_source_id: point.point_source_id,
            gps_time: point.gps_time,
            color: point
                .color
                .map(|color| [color.red, color.green, color.blue]),
            nir: point.nir,
            is_synthetic: point.is_synthetic,
            is_key_point: point.is_key_point,
            is_withheld: point.is_withheld,
            is_overlap: point.is_overlap,
        }
    }
}

/// Memory limits for building a display sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleOptions {
    /// Maximum number of points retained in memory for display.
    pub max_points: usize,
    /// Maximum number of source records decoded in one batch.
    pub chunk_size: usize,
}

impl Default for SampleOptions {
    fn default() -> Self {
        Self {
            max_points: 1_000_000,
            chunk_size: 65_536,
        }
    }
}

/// A bounded, approximately uniform sample of a source cloud.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointSample {
    pub metadata: CloudMetadata,
    pub points: Vec<SamplePoint>,
    /// Every `stride`th source point was selected.
    pub stride: u64,
    /// Number of source records decoded to produce the sample.
    pub scanned_points: u64,
}

/// Reads only the LAS/LAZ header and metadata.
pub fn inspect(path: impl AsRef<Path>) -> Result<CloudMetadata> {
    let reader = Reader::from_path(path)?;
    CloudMetadata::from_header(reader.header())
}

/// Streams a LAS/LAZ file and retains at most `options.max_points` records.
///
/// The source index is retained so a selected display point can be recorded as
/// a sparse classification edit and applied to the correct full-resolution
/// record during export.
pub fn sample(path: impl AsRef<Path>, options: SampleOptions) -> Result<PointSample> {
    if options.max_points == 0 {
        return Err(Error::InvalidLimit("max_points"));
    }
    if options.chunk_size == 0 {
        return Err(Error::InvalidLimit("chunk_size"));
    }

    let mut reader = Reader::from_path(path)?;
    let metadata = CloudMetadata::from_header(reader.header())?;
    let max_points = u64::try_from(options.max_points).unwrap_or(u64::MAX);
    let stride = metadata.point_count.max(1).div_ceil(max_points).max(1);
    let mut points = Vec::with_capacity(
        options
            .max_points
            .min(usize::try_from(metadata.point_count).unwrap_or(usize::MAX)),
    );
    let mut source_index = 0_u64;

    while source_index < metadata.point_count && points.len() < options.max_points {
        let remaining = metadata.point_count - source_index;
        let request = remaining.min(options.chunk_size as u64);
        let point_data = reader.read_points(request)?;
        if point_data.is_empty() {
            break;
        }

        for point in point_data.points() {
            let point = point?;
            if source_index % stride == 0 && points.len() < options.max_points {
                points.push(SamplePoint::from_point(source_index, point));
            }
            source_index += 1;
        }
    }

    Ok(PointSample {
        metadata,
        points,
        stride,
        scanned_points: source_index,
    })
}

/// Sparse, transactional point-classification changes.
///
/// Only source indices that were modified consume memory. Undo restores the
/// previous sparse state; source files are never modified in place.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassificationEdits {
    changes: BTreeMap<u64, u8>,
    #[serde(skip)]
    history: Vec<Vec<(u64, Option<u8>)>>,
}

impl ClassificationEdits {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, u8)> + '_ {
        self.changes
            .iter()
            .map(|(&source_index, &classification)| (source_index, classification))
    }

    pub fn classification_for(&self, source_index: u64) -> Option<u8> {
        self.changes.get(&source_index).copied()
    }

    /// Applies one classification to a set of source indices as one undoable
    /// transaction. Duplicate indices are collapsed.
    pub fn reclassify(
        &mut self,
        source_indices: impl IntoIterator<Item = u64>,
        classification: u8,
    ) -> usize {
        let unique: BTreeSet<_> = source_indices.into_iter().collect();
        if unique.is_empty() {
            return 0;
        }

        let mut previous = Vec::with_capacity(unique.len());
        for source_index in unique {
            let old = self.changes.insert(source_index, classification);
            previous.push((source_index, old));
        }
        let changed = previous.len();
        self.history.push(previous);
        changed
    }

    /// Undoes the most recent reclassification transaction.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.history.pop() else {
            return false;
        };
        for (source_index, classification) in previous {
            match classification {
                Some(classification) => {
                    self.changes.insert(source_index, classification);
                }
                None => {
                    self.changes.remove(&source_index);
                }
            }
        }
        true
    }

    pub fn clear(&mut self) {
        if self.changes.is_empty() {
            return;
        }
        let previous = self
            .changes
            .iter()
            .map(|(&source_index, &classification)| (source_index, Some(classification)))
            .collect();
        self.changes.clear();
        self.history.push(previous);
    }
}

/// Result of a full-resolution export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportStats {
    pub points_read: u64,
    pub points_written: u64,
    pub points_reclassified: u64,
}

/// Streams the source cloud to a new LAS/LAZ and applies sparse edits.
///
/// The source header is cloned, including CRS VLRs and extra-byte definitions.
/// The output is written to an adjacent temporary file and renamed only after
/// the writer closes successfully. Existing outputs and in-place replacement
/// are refused.
pub fn export_with_edits(
    input: impl AsRef<Path>,
    output: impl AsRef<Path>,
    edits: &ClassificationEdits,
) -> Result<ExportStats> {
    const CHUNK_SIZE: u64 = 65_536;

    let input = input.as_ref();
    let output = output.as_ref();
    validate_output_path(input, output)?;

    let mut reader = Reader::from_path(input)?;
    let header = reader.header().clone();
    let point_count = header.number_of_points();
    let temporary = temporary_output_path(output);
    let mut temporary_guard = TemporaryOutput::new(temporary.clone());
    let mut writer = Writer::from_path(&temporary, header)?;
    let mut stats = ExportStats::default();

    while stats.points_read < point_count {
        let point_data = reader.read_points((point_count - stats.points_read).min(CHUNK_SIZE))?;
        if point_data.is_empty() {
            break;
        }

        for point in point_data.points() {
            let mut point = point?;
            if let Some(classification) = edits.classification_for(stats.points_read) {
                apply_classification(&mut point, classification)?;
                stats.points_reclassified += 1;
            }
            writer.write_point(point)?;
            stats.points_read += 1;
            stats.points_written += 1;
        }
    }

    writer.close()?;
    drop(writer);
    fs::rename(&temporary, output)?;
    temporary_guard.commit();
    Ok(stats)
}

fn apply_classification(point: &mut Point, classification: u8) -> Result<()> {
    if classification == 12 {
        // LAS 1.4 represents overlap as a flag. las-rs also maps legacy class
        // 12 to Unclassified + this flag when reading old point formats.
        point.classification = Classification::Unclassified;
        point.is_overlap = true;
    } else {
        point.classification = Classification::new(classification)?;
    }
    Ok(())
}

fn validate_output_path(input: &Path, output: &Path) -> Result<()> {
    let supported = output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
        });
    if !supported {
        return Err(Error::UnsupportedExtension(output.to_owned()));
    }
    if output.exists() {
        return Err(Error::OutputExists(output.to_owned()));
    }

    let input_absolute = absolute_path(input)?;
    let output_absolute = absolute_path(output)?;
    if input_absolute == output_absolute {
        return Err(Error::SameInputAndOutput(output.to_owned()));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let extension = output
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("las");
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("point-cloud");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    output.with_file_name(format!(
        ".{stem}.ocs-part-{}-{nonce}.{extension}",
        std::process::id()
    ))
}

struct TemporaryOutput {
    path: PathBuf,
    committed: bool,
}

impl TemporaryOutput {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use las::{point::Format, Builder};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ocs-pointcloud-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: impl AsRef<Path>) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn create_cloud(path: &Path, count: u64) {
        let mut builder = Builder::default();
        builder.point_format = Format::new(3).unwrap();
        let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
        for index in 0..count {
            writer
                .write_point(Point {
                    x: 1000.0 + index as f64,
                    y: 2000.0 + index as f64 * 2.0,
                    z: 100.0 + index as f64 * 0.5,
                    intensity: (100 + index) as u16,
                    classification: Classification::new((index % 6) as u8).unwrap(),
                    return_number: 1,
                    number_of_returns: 1,
                    gps_time: Some(50_000.0 + index as f64),
                    color: Some(las::Color::new(index as u16, 20, 30)),
                    ..Default::default()
                })
                .unwrap();
        }
        writer.close().unwrap();
    }

    #[test]
    fn metadata_and_sample_are_bounded_for_las_and_laz() {
        let directory = TestDirectory::new();
        for name in ["sample.las", "sample.laz"] {
            let path = directory.join(name);
            create_cloud(&path, 101);

            let metadata = inspect(&path).unwrap();
            assert_eq!(101, metadata.point_count);
            assert_eq!(3, metadata.point_format);
            assert_eq!(name.ends_with(".laz"), metadata.compressed);
            assert_eq!([1000.0, 2000.0, 100.0], metadata.bounds_min);

            let sample = sample(
                &path,
                SampleOptions {
                    max_points: 10,
                    chunk_size: 7,
                },
            )
            .unwrap();
            assert!(sample.points.len() <= 10);
            assert_eq!(11, sample.stride);
            assert_eq!(0, sample.points[0].source_index);
            assert_eq!(11, sample.points[1].source_index);
            assert_eq!(Some(50_000.0), sample.points[0].gps_time);
        }
    }

    #[test]
    fn sparse_edits_are_transactional_and_undoable() {
        let mut edits = ClassificationEdits::default();
        assert_eq!(2, edits.reclassify([2, 4, 4], 2));
        assert_eq!(Some(2), edits.classification_for(4));
        assert_eq!(2, edits.reclassify([4, 8], 6));
        assert_eq!(Some(6), edits.classification_for(4));
        assert!(edits.undo());
        assert_eq!(Some(2), edits.classification_for(4));
        assert_eq!(None, edits.classification_for(8));
        edits.clear();
        assert!(edits.is_empty());
        assert!(edits.undo());
        assert_eq!(Some(2), edits.classification_for(2));
        assert_eq!(Some(2), edits.classification_for(4));
    }

    #[test]
    fn export_preserves_points_and_applies_edits_to_las_and_laz() {
        let directory = TestDirectory::new();
        for extension in ["las", "laz"] {
            let input = directory.join(format!("input.{extension}"));
            let output = directory.join(format!("output.{extension}"));
            create_cloud(&input, 20);
            let input_metadata = inspect(&input).unwrap();

            let mut edits = ClassificationEdits::default();
            edits.reclassify([1, 7], 2);
            edits.reclassify([9], 12);
            let stats = export_with_edits(&input, &output, &edits).unwrap();
            assert_eq!(20, stats.points_read);
            assert_eq!(20, stats.points_written);
            assert_eq!(3, stats.points_reclassified);

            let output_metadata = inspect(&output).unwrap();
            assert_eq!(input_metadata.point_count, output_metadata.point_count);
            assert_eq!(input_metadata.point_format, output_metadata.point_format);
            assert_eq!(input_metadata.bounds_min, output_metadata.bounds_min);
            assert_eq!(input_metadata.bounds_max, output_metadata.bounds_max);

            let mut reader = Reader::from_path(&output).unwrap();
            let points: Vec<_> = reader
                .read_all()
                .unwrap()
                .points()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert_eq!(2, u8::from(points[1].classification));
            assert_eq!(2, u8::from(points[7].classification));
            assert_eq!(1, u8::from(points[9].classification));
            assert!(points[9].is_overlap);
            assert_eq!(110, points[10].intensity);
            assert_eq!(Some(50_010.0), points[10].gps_time);
            assert_eq!(Some(las::Color::new(10, 20, 30)), points[10].color);
        }
    }

    #[test]
    fn export_refuses_overwrite_and_bad_extensions() {
        let directory = TestDirectory::new();
        let input = directory.join("input.las");
        create_cloud(&input, 1);
        assert!(matches!(
            export_with_edits(&input, &input, &ClassificationEdits::default()),
            Err(Error::OutputExists(_))
        ));
        assert!(matches!(
            export_with_edits(
                &input,
                directory.join("output.txt"),
                &ClassificationEdits::default()
            ),
            Err(Error::UnsupportedExtension(_))
        ));
    }
}
