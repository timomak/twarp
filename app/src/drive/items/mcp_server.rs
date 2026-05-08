use super::{WarpDriveItem, WarpDriveItemId};
// twarp: 2c-d — AI MCP CloudMCPServer deleted; stub.
// twarp: 2c-d — CloudObjectMetadata isn't Default; use Option
#[derive(Default, Clone)]
pub struct CloudMCPServer {
    // twarp: 2c-d — fields used by callers
    pub metadata: Option<CloudObjectMetadata>,
}
#[allow(dead_code)]
impl CloudMCPServer {
    pub fn model(&self) -> CloudMCPServerModelStub {
        CloudMCPServerModelStub::default()
    }
}
#[derive(Default)]
pub struct CloudMCPServerModelStub {
    pub string_model: CloudMCPServerStringModelStub,
}
#[derive(Default)]
pub struct CloudMCPServerStringModelStub {
    pub name: String,
}
use crate::{
    appearance::Appearance,
    cloud_object::CloudObjectMetadata,
    drive::{index::DriveIndexAction, CloudObjectTypeAndId, DriveObjectType},
    themes::theme::Fill,
};
use warpui::{elements::MouseStateHandle, AppContext, Element};

#[derive(Clone)]
pub struct WarpDriveMCPServer {
    id: CloudObjectTypeAndId,
    mcp_server: CloudMCPServer,
}

impl WarpDriveMCPServer {
    pub fn new(id: CloudObjectTypeAndId, mcp_server: CloudMCPServer) -> Self {
        Self { id, mcp_server }
    }
}

impl WarpDriveItem for WarpDriveMCPServer {
    fn display_name(&self) -> Option<String> {
        Some(self.mcp_server.model().string_model.name.clone())
    }
    fn metadata(&self) -> Option<&CloudObjectMetadata> {
        // twarp: 2c-d — metadata is now Option<CloudObjectMetadata>
        self.mcp_server.metadata.as_ref()
    }

    fn object_type(&self) -> Option<DriveObjectType> {
        Some(DriveObjectType::MCPServer)
    }

    fn secondary_icon(&self, _color: Option<Fill>) -> Option<Box<dyn Element>> {
        None
    }

    fn click_action(&self) -> Option<DriveIndexAction> {
        Some(DriveIndexAction::OpenMCPServerCollection)
    }

    fn preview(&self, _appearance: &Appearance) -> Option<Box<dyn Element>> {
        // TODO
        None
    }

    fn warp_drive_id(&self) -> WarpDriveItemId {
        WarpDriveItemId::Object(self.id)
    }

    fn sync_status_icon(
        &self,
        sync_queue_is_dequeueing: bool,
        hover_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        self.mcp_server
            .metadata
            .as_ref()?
            .pending_changes_statuses
            .render_icon(sync_queue_is_dequeueing, hover_state, appearance)
    }

    fn action_summary(&self, _app: &AppContext) -> Option<String> {
        None
    }

    fn clone_box(&self) -> Box<dyn WarpDriveItem> {
        Box::new(self.clone())
    }
}
