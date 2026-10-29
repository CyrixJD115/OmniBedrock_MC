//! server.properties access helper shared by the properties endpoints.
use omnibedrock_core::{error::Result, files::safe_path};
use omnibedrock_server::instances::Instance;
use std::path::Path as FsPath;
use std::sync::Arc;

pub(super) async fn properties_text(svc: &Arc<Instance>) -> Result<String> {
    match tokio::fs::read_to_string(
        safe_path(
            &svc.config.bedrock_server_dir,
            FsPath::new("server.properties"),
        )
        .await?,
    )
    .await
    {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}
