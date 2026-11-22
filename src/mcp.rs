use std::sync::{Arc, Mutex};

use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::{desktop::Input, image::Mode, session::Session};

#[derive(Clone)]
pub struct McpServer {
    session: Arc<Mutex<Session>>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
struct Observe {
    /// A window handle returned by list_windows.
    window: u64,
    #[serde(default)]
    mode: Mode,
    /// Follow a foreground dialog owned by the requested window.
    #[serde(default = "yes")]
    follow_owned: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, JsonSchema)]
struct Point {
    observation_id: String,
    /// The index of a PNG in the observation, starting at zero.
    image: usize,
    /// X in the selected image's own pixels.
    x: u32,
    /// Y in the selected image's own pixels.
    y: u32,
}

#[derive(Deserialize, JsonSchema)]
struct Scroll {
    #[serde(flatten)]
    point: Point,
    /// Wheel ticks. Positive scrolls up and negative scrolls down.
    ticks: i32,
}

#[derive(Deserialize, JsonSchema)]
struct Fill {
    #[serde(flatten)]
    point: Point,
    text: String,
    #[serde(default)]
    replace: bool,
}

#[derive(Deserialize, JsonSchema)]
struct Text {
    observation_id: String,
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct Key {
    observation_id: String,
    chord: String,
}

#[tool_router]
impl McpServer {
    pub fn new(session: Session) -> Self {
        Self {
            session: Arc::new(Mutex::new(session)),
            tool_router: Self::tool_router(),
        }
    }

    fn reply(
        &self,
        execute: impl FnOnce(&mut Session) -> Result<CallToolResult>,
    ) -> CallToolResult {
        match self.session.lock() {
            Ok(mut session) => match execute(&mut session) {
                Ok(reply) => reply,
                Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
            },
            Err(_) => CallToolResult::error(vec![ContentBlock::text(
                "The desktop session is unavailable.",
            )]),
        }
    }

    #[tool(
        description = "List visible, non-minimized desktop windows and their window handles.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn list_windows(&self) -> CallToolResult {
        self.reply(|session| {
            Ok(CallToolResult::structured(
                json!({"result": session.windows()?}),
            ))
        })
    }

    #[tool(
        description = "Observe a window or its foreground owned dialog. Returns JSON metadata followed by PNG images in image-index order."
    )]
    fn observe_window(&self, Parameters(args): Parameters<Observe>) -> CallToolResult {
        self.reply(|session| {
            let observed = session.observe(args.window, args.mode, args.follow_owned)?;
            let mut parts = vec![ContentBlock::text(serde_json::to_string(
                &observed.metadata,
            )?)];
            parts.extend(
                observed
                    .pngs
                    .into_iter()
                    .map(|png| ContentBlock::image(STANDARD.encode(png), "image/png")),
            );
            Ok(CallToolResult::success(parts))
        })
    }

    #[tool(
        description = "Convert an image-local point to global screen coordinates for the current platform.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn map_point(&self, Parameters(point): Parameters<Point>) -> CallToolResult {
        self.reply(|session| {
            let screen = session.map(&point.observation_id, point.image, point.x, point.y)?;
            Ok(CallToolResult::structured(serde_json::to_value(screen)?))
        })
    }

    #[tool(description = "Left-click a visible point in the observed foreground window.")]
    fn click(&self, Parameters(point): Parameters<Point>) -> CallToolResult {
        self.reply(|session| {
            let screen = session.map(&point.observation_id, point.image, point.x, point.y)?;
            session.input(&point.observation_id, Input::Click { point: screen })?;
            Ok(input_sent())
        })
    }

    #[tool(description = "Scroll at a visible point in the observed foreground window.")]
    fn scroll(&self, Parameters(args): Parameters<Scroll>) -> CallToolResult {
        self.reply(|session| {
            let Point {
                observation_id,
                image,
                x,
                y,
            } = args.point;
            let screen = session.map(&observation_id, image, x, y)?;
            session.input(
                &observation_id,
                Input::Scroll {
                    point: screen,
                    ticks: args.ticks,
                },
            )?;
            Ok(input_sent())
        })
    }

    #[tool(
        description = "Click a known text field and type in one action. Set replace to select the existing text first."
    )]
    fn fill(&self, Parameters(args): Parameters<Fill>) -> CallToolResult {
        self.reply(|session| {
            let Point {
                observation_id,
                image,
                x,
                y,
            } = args.point;
            let screen = session.map(&observation_id, image, x, y)?;
            session.input(
                &observation_id,
                Input::Fill {
                    point: screen,
                    text: &args.text,
                    replace: args.replace,
                },
            )?;
            Ok(input_sent())
        })
    }

    #[tool(
        description = "Type plain Unicode text into the current focus of the observed foreground window."
    )]
    fn type_text(&self, Parameters(args): Parameters<Text>) -> CallToolResult {
        self.reply(|session| {
            session.input(&args.observation_id, Input::Text(&args.text))?;
            Ok(input_sent())
        })
    }

    #[tool(description = "Send a key or shortcut such as enter, tab, f7, ctrl+a, or shift+end.")]
    fn press_key(&self, Parameters(args): Parameters<Key>) -> CallToolResult {
        self.reply(|session| {
            session.input(&args.observation_id, Input::Key(&args.chord))?;
            Ok(input_sent())
        })
    }
}

fn input_sent() -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(
        "Input sent. Observe the window again to verify the result.",
    )])
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("VisionCache", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Observe the selected window before sending input. Keep using its original handle to follow owned dialogs. \
                 Image 0 shows the whole client area. Any other image is a native-resolution crop. \
                 Use the latest observation_id and coordinates relative to the selected image. \
                 Observe again after each input. Request mode=full to read fine details.",
            )
    }
}
