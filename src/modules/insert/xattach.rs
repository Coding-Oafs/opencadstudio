// XATTACH command — attach an external DWG/DXF file as an XREF block
// and insert it at a picked point.
//
// Workflow:
//   Step 1 (text input): user types the file path (or the file-picker
//           message has already supplied it).
//   Step 2 (point pick): user clicks the insertion point.
//   Result: BlockRecord + Block entities are created with is_xref=true,
//           then an INSERT entity is committed.

use acadrust::entities::{Block, BlockEnd, Insert};
use acadrust::tables::block_record::{BlockFlags, BlockRecord};
use acadrust::types::Vector3;
use acadrust::EntityType;
use glam::DVec3;

use crate::t;

use crate::command::{CadCommand, CmdResult, WorkingPlane};
use crate::modules::{IconKind, ModuleEvent, ToolDef};
use crate::scene::model::wire_model::WireModel;
use crate::scene::Scene;

pub fn tool() -> ToolDef {
    ToolDef {
        id: "XATTACH",
        label: "Attach XREF",
        icon: IconKind::Svg(include_bytes!("../../../assets/icons/blocks/insert.svg")),
        event: ModuleEvent::Command("XATTACH".to_string()),
    }
}

pub struct XAttachCommand {
    path: String,
    block_name: String,
    plane: WorkingPlane,
}

impl XAttachCommand {
    /// Create an XATTACH command with a path already filled in (from file-picker).
    pub fn with_path(path: String) -> Self {
        let block_name = path_to_block_name(&path);
        Self {
            path,
            block_name,
            plane: WorkingPlane::default(),
        }
    }
}

impl CadCommand for XAttachCommand {
    fn set_working_plane(&mut self, plane: WorkingPlane) {
        self.plane = plane;
    }

    fn name(&self) -> &'static str {
        "XATTACH"
    }

    fn prompt(&self) -> String {
        t!(
            "XATTACH  Specify insertion point for \"%{name}\":",
            name = self.block_name
        )
        .into_owned()
    }

    fn on_point(&mut self, pt: DVec3) -> CmdResult {
        // We return the INSERT entity; the command handler in commands.rs
        // calls `prepare_xref_block` on the scene before committing.
        let point = self.plane.to_local(pt);
        CmdResult::CommitAndExit(self.plane.place_entity(EntityType::Insert(Insert::new(
            self.block_name.clone(),
            Vector3::new(point.x, point.y, point.z),
        ))))
    }

    fn on_enter(&mut self) -> CmdResult {
        CmdResult::Cancel
    }

    fn on_preview_wires(&mut self, _pt: DVec3) -> Vec<WireModel> {
        vec![]
    }

    fn xattach_path(&self) -> Option<String> {
        Some(self.path.clone())
    }
}

/// Derive a block name from the file path: take the file stem, uppercase it.
pub fn path_to_block_name(path: &str) -> String {
    let p = std::path::Path::new(path);
    p.file_stem()
        .map(|s| s.to_string_lossy().to_uppercase())
        .unwrap_or_else(|| "XREF".to_string())
}

/// Create the XREF BlockRecord + Block/EndBlock entities in the scene document
/// for a given file path.  Returns the block name.
///
/// This must be called before committing the INSERT so that the block
/// definition exists when the renderer looks it up.
pub fn prepare_xref_block(scene: &mut Scene, path: &str) -> String {
    let block_name = path_to_block_name(path);

    // If a BlockRecord already exists with this name, skip creation.
    if scene.document.block_records.get(&block_name).is_some() {
        return block_name;
    }

    // Create the BlockRecord.
    let mut br = BlockRecord::new(&block_name);
    br.handle = scene.document.allocate_handle();
    br.flags = BlockFlags {
        is_xref: true,
        is_xref_overlay: false,
        anonymous: false,
        has_attributes: false,
        is_external: false,
    };
    br.xref_path = path.to_string();
    let _ = scene.document.block_records.add(br);

    // Create BLOCK entity.
    let b = Block::new(&block_name, Vector3::zero()).with_xref_path(path);
    let _ = scene.document.add_entity(EntityType::Block(b));
    let _ = scene
        .document
        .add_entity(EntityType::BlockEnd(BlockEnd::new()));

    // Resolve the XREF content immediately.
    let path_buf = std::path::PathBuf::from(path);
    if let Some(base_dir) = path_buf.parent() {
        let _ = crate::io::xref::resolve_xrefs(&mut scene.document, base_dir);
    }

    block_name
}
