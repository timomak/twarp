pub mod util;

// twarp: 2c-d — AI ambient/execution_profiles/facts/mcp deleted; stubs.
pub struct CloudScheduledAmbientAgentModel;
pub struct CloudAIExecutionProfileModel;
pub struct CloudAIFactModel;
pub struct CloudTemplatableMCPServerModel;
pub struct CloudMCPServerModel;
use crate::{
    cloud_object::{
        model::generic_string_model::GenericStringObjectId, GenericServerObject,
        RevisionAndLastEditor, ServerFolder, ServerObject, UpdateCloudObjectResult,
    },
    env_vars::CloudEnvVarCollectionModel,
    server::{graphql::get_user_facing_error_message, ids::ServerId},
    settings::cloud_preferences::CloudPreferenceModel,
    workflows::workflow_enum::CloudWorkflowEnumModel,
};

use anyhow::{bail, Result};
use warp_graphql::{
    generic_string_object::GenericStringObjectFormat,
    mutations::update_generic_string_object::{
        GenericStringObjectUpdate, UpdateGenericStringObjectResult,
    },
    object::ObjectUpdateSuccess,
};

impl TryFrom<UpdateGenericStringObjectResult> for UpdateCloudObjectResult<Box<dyn ServerObject>> {
    type Error = anyhow::Error;

    fn try_from(value: UpdateGenericStringObjectResult) -> std::result::Result<Self, Self::Error> {
        match value {
            UpdateGenericStringObjectResult::UpdateGenericStringObjectOutput(output) => {
                match output.update {
                    GenericStringObjectUpdate::ObjectUpdateSuccess(success) => {
                        Ok(UpdateCloudObjectResult::Success {
                            revision_and_editor: RevisionAndLastEditor {
                                revision: success.revision_ts.into(),
                                last_editor_uid: Some(success.last_editor_uid.into_inner()),
                            },
                        })
                    }
                    GenericStringObjectUpdate::GenericStringObjectUpdateRejected(rejected) => {
                        let boxed: Box<dyn ServerObject> = match rejected
                            .conflicting_generic_string_object
                            .format
                        {
                            GenericStringObjectFormat::JsonEnvVarCollection => {
                                let gso = GenericServerObject::<
                                    GenericStringObjectId,
                                    CloudEnvVarCollectionModel,
                                >::try_from_graphql_fields(
                                    ServerId::from_string_lossy(
                                        rejected
                                            .conflicting_generic_string_object
                                            .metadata
                                            .uid
                                            .inner(),
                                    ),
                                    Some(
                                        rejected.conflicting_generic_string_object.serialized_model,
                                    ),
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .try_into()?,
                                    rejected
                                        .conflicting_generic_string_object
                                        .permissions
                                        .try_into()?,
                                )?;
                                let boxed: Box<dyn ServerObject> = Box::new(gso);
                                boxed
                            }
                            GenericStringObjectFormat::JsonPreference => {
                                let gso = GenericServerObject::<
                                    GenericStringObjectId,
                                    CloudPreferenceModel,
                                >::try_from_graphql_fields(
                                    ServerId::from_string_lossy(
                                        rejected
                                            .conflicting_generic_string_object
                                            .metadata
                                            .uid
                                            .inner(),
                                    ),
                                    Some(
                                        rejected.conflicting_generic_string_object.serialized_model,
                                    ),
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .try_into()?,
                                    rejected
                                        .conflicting_generic_string_object
                                        .permissions
                                        .try_into()?,
                                )?;
                                let boxed: Box<dyn ServerObject> = Box::new(gso);
                                boxed
                            }
                            GenericStringObjectFormat::JsonWorkflowEnum => {
                                let gso = GenericServerObject::<
                                    GenericStringObjectId,
                                    CloudWorkflowEnumModel,
                                >::try_from_graphql_fields(
                                    ServerId::from_string_lossy(
                                        rejected
                                            .conflicting_generic_string_object
                                            .metadata
                                            .uid
                                            .inner(),
                                    ),
                                    Some(
                                        rejected.conflicting_generic_string_object.serialized_model,
                                    ),
                                    rejected
                                        .conflicting_generic_string_object
                                        .metadata
                                        .try_into()?,
                                    rejected
                                        .conflicting_generic_string_object
                                        .permissions
                                        .try_into()?,
                                )?;
                                let boxed: Box<dyn ServerObject> = Box::new(gso);
                                boxed
                            }
                            // twarp: 2c-d — AI cloud objects deleted; bail out on these variants.
                            GenericStringObjectFormat::JsonAIFact
                            | GenericStringObjectFormat::JsonAIExecutionProfile
                            | GenericStringObjectFormat::JsonMCPServer
                            | GenericStringObjectFormat::JsonTemplatableMCPServer
                            | GenericStringObjectFormat::JsonScheduledAmbientAgent => {
                                anyhow::bail!(
                                    "AI cloud objects no longer supported client-side"
                                )
                            }
                            GenericStringObjectFormat::JsonCloudEnvironment => {
                                anyhow::bail!(
                                    "CloudEnvironment objects are no longer supported client-side"
                                )
                            }
                        };
                        Ok(UpdateCloudObjectResult::Rejected { object: boxed })
                    }
                    GenericStringObjectUpdate::Unknown => {
                        bail!("update generic string object response has unknown variant")
                    }
                }
            }
            UpdateGenericStringObjectResult::UserFacingError(e) => {
                bail!(get_user_facing_error_message(e))
            }
            UpdateGenericStringObjectResult::Unknown => {
                bail!("update generic string object response has unknown variant")
            }
        }
    }
}

impl TryFrom<ObjectUpdateSuccess> for UpdateCloudObjectResult<ServerFolder> {
    type Error = anyhow::Error;

    fn try_from(value: ObjectUpdateSuccess) -> Result<Self, Self::Error> {
        Ok(UpdateCloudObjectResult::Success {
            revision_and_editor: RevisionAndLastEditor {
                revision: value.revision_ts.into(),
                last_editor_uid: Some(value.last_editor_uid.into_inner()),
            },
        })
    }
}
