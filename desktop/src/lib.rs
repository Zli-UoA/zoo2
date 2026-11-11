pub mod app;
pub mod input;
pub mod login;
pub mod mfm_view;
pub mod theme;
pub mod view;

use std::sync::Arc;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};
use zoo2_client_core::config::Config;

pub use app::AppState;
pub use view::WorkspaceView;

actions!(zoo2_desktop_app, [Quit]);

#[derive(Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
    tokio_handle: tokio::runtime::Handle,
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        let tokio_handle = tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
            let rt =
                tokio::runtime::Runtime::new().expect("Tokio ランタイムの初期化に失敗しました");
            let handle = rt.handle().clone();
            Box::leak(Box::new(rt));
            handle
        });
        Self {
            client: reqwest::Client::new(),
            tokio_handle,
        }
    }
}

impl ReqwestHttpClient {
    pub fn new(tokio_handle: tokio::runtime::Handle) -> Self {
        Self {
            client: reqwest::Client::new(),
            tokio_handle,
        }
    }
}

impl gpui::http_client::HttpClient for ReqwestHttpClient {
    fn type_name(&self) -> &'static str {
        "ReqwestHttpClient"
    }

    fn user_agent(&self) -> Option<&gpui::http_client::http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&gpui::http_client::Url> {
        None
    }

    fn send(
        &self,
        req: gpui::http_client::Request<gpui::http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<
        'static,
        anyhow::Result<gpui::http_client::Response<gpui::http_client::AsyncBody>>,
    > {
        let client = self.client.clone();
        let handle = self.tokio_handle.clone();
        Box::pin(async move {
            let join_handle = handle.spawn(async move {
                let url = req.uri().to_string();
                let method = req.method().clone();
                let mut reqwest_req = client.request(
                    reqwest::Method::from_bytes(method.as_str().as_bytes())?,
                    &url,
                );
                for (name, val) in req.headers() {
                    reqwest_req = reqwest_req.header(name.as_str(), val.as_bytes());
                }
                let res = reqwest_req.send().await?;
                let status = res.status().as_u16();
                let mut builder = gpui::http_client::Response::builder().status(status);
                for (name, val) in res.headers() {
                    builder = builder.header(name.as_str(), val.as_bytes());
                }
                let bytes = res.bytes().await?;
                let response = builder.body(gpui::http_client::AsyncBody::from(bytes))?;
                Ok(response)
            });
            join_handle.await?
        })
    }
}

pub fn run(config: Config) {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Tokio ランタイムの起動に失敗しました"),
    );

    let http_client = Arc::new(ReqwestHttpClient::new(runtime.handle().clone()));

    Application::new()
        .with_http_client(http_client)
        .run(move |cx: &mut App| {
            input::bind_input_keys(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);

            let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
            let state = AppState::spawn_entity(config, runtime.clone(), cx);

            if let Err(err) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("zoo2".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |_, cx| cx.new(|cx| WorkspaceView::new(state, cx)),
            ) {
                eprintln!("ウィンドウを開けませんでした: {err:#}");
                std::process::exit(1);
            }
            cx.activate(true);
        });
}
