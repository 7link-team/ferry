//! Serves the built web UI from inside the binary so one executable is the
//! whole deployment in Docker mode and the Tauri sidecar in desktop mode.

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../dist"]
struct Web;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = Web::get(path).or_else(|| Web::get("index.html"));
    match file {
        // A client-side route is not a file; the shell resolves it.
        Some(file) => {
            let mime = mime_guess::from_path(if Web::get(path).is_some() { path } else { "index.html" })
                .first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response()
        }
        None => (StatusCode::NOT_FOUND, "界面文件未构建，请先运行 npm run build。").into_response(),
    }
}
