use super::*;
use ::image::{DynamicImage, ImageReader, Limits, Rgba, imageops::FilterType};
use ratatui_image::{
    Resize, StatefulImage,
    picker::{Picker, ProtocolType},
    protocol::{StatefulProtocol, StatefulProtocolType},
};
use std::{collections::VecDeque, io::Cursor};

const MAX_BYTES: usize = 4 * 1024 * 1024;

fn terminal_protocol(
    term: &str,
    program: &str,
    forced: Option<&str>,
    known_cell_size: bool,
) -> ProtocolType {
    match forced {
        Some("kitty") => ProtocolType::Kitty,
        Some("sixel") => ProtocolType::Sixel,
        Some("iterm2") => ProtocolType::Iterm2,
        Some(_) => ProtocolType::Halfblocks,
        None if !known_cell_size => ProtocolType::Halfblocks,
        None if term == "xterm-kitty" => ProtocolType::Kitty,
        None if program == "iTerm.app" || program == "WezTerm" => ProtocolType::Iterm2,
        None if term.ends_with("-sixel") => ProtocolType::Sixel,
        None => ProtocolType::Halfblocks,
    }
}

pub(super) fn terminal_picker() -> Picker {
    // from_query_stdio's timeout leaves a blocking stdin reader behind in 10.0.2.
    // Never let capability detection compete with the TUI's sole event reader.
    let cell_size = ratatui::crossterm::terminal::window_size()
        .ok()
        .and_then(|size| {
            let width = size.width.checked_div(size.columns)?;
            let height = size.height.checked_div(size.rows)?;
            (width > 0 && height > 0).then_some((width, height))
        });
    let protocol = terminal_protocol(
        &std::env::var("TERM").unwrap_or_default(),
        &std::env::var("TERM_PROGRAM").unwrap_or_default(),
        std::env::var("JORIK_ARTWORK_PROTOCOL").ok().as_deref(),
        cell_size.is_some(),
    );
    // This is the library's only constructor that accepts a cell size without reading stdin.
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(cell_size.unwrap_or((10, 20)));
    picker.set_protocol_type(protocol);
    picker
}

pub(super) struct Artwork {
    url: Option<String>,
    generation: u64,
    pending: bool,
    pub loading: bool,
    picker: Picker,
    background: Color,
    image: Option<DynamicImage>,
    protocol: Option<StatefulProtocol>,
    cache: VecDeque<(String, DynamicImage)>,
    pub last_area: Option<Rect>,
}

pub(super) struct Request {
    url: String,
    generation: u64,
}

impl Artwork {
    pub fn new() -> Self {
        Self {
            url: None,
            generation: 0,
            pending: false,
            loading: false,
            picker: Picker::halfblocks(),
            background: Color::Reset,
            image: None,
            protocol: None,
            cache: VecDeque::new(),
            last_area: None,
        }
    }

    pub fn clear(&mut self) {
        self.set_url(None);
        self.cache.clear();
    }

    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = picker;
        self.background = Color::Reset;
        self.protocol = self
            .image
            .clone()
            .map(|image| self.picker.new_resize_protocol(image));
    }

    pub fn set_url(&mut self, url: Option<&str>) {
        if self.url.as_deref() == url {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.url = url.map(str::to_owned);
        self.image = url.and_then(|url| {
            self.cache
                .iter()
                .find(|(key, _)| key == url)
                .map(|(_, image)| image.clone())
        });
        self.protocol = self
            .image
            .clone()
            .map(|image| self.picker.new_resize_protocol(image));
        self.pending = self.url.is_some() && self.image.is_none();
        self.loading = self.pending;
    }

    pub fn take_request(&mut self) -> Option<Request> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        Some(Request {
            url: self.url.clone()?,
            generation: self.generation,
        })
    }

    pub fn retry(&mut self) {
        if self.url.is_some() && self.image.is_none() && !self.loading {
            self.pending = true;
            self.loading = true;
        }
    }

    #[cfg(test)]
    pub fn install_fixture(&mut self, image: DynamicImage) {
        if let Some(request) = self.take_request() {
            self.accept(request, Ok(image));
        }
    }

    fn accept(&mut self, request: Request, result: Result<DynamicImage>) {
        if self.generation != request.generation
            || self.url.as_deref() != Some(request.url.as_str())
        {
            return;
        }
        self.loading = false;
        if let Ok(image) = result {
            self.cache.push_front((request.url, image.clone()));
            self.cache.truncate(4);
            self.protocol = Some(self.picker.new_resize_protocol(image.clone()));
            self.image = Some(image);
        }
    }

    pub fn rows_for_width(&self, width: u16) -> u16 {
        let (fw, fh) = self.picker.font_size();
        let (iw, ih) = self
            .image
            .as_ref()
            .map(|image| (image.width(), image.height()))
            .unwrap_or((1, 1));
        (u64::from(width) * u64::from(fw.max(1)) * u64::from(ih))
            .div_ceil(u64::from(fh.max(1)) * u64::from(iw.max(1)))
            .min(u64::from(u16::MAX)) as u16
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        if area.is_empty() {
            return;
        }
        if self.background != theme.surface() {
            self.background = theme.surface();
            let (r, g, b) = match self.background {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (28, 34, 50),
            };
            self.picker.set_background_color(Rgba([r, g, b, 255]));
            self.protocol = self
                .image
                .clone()
                .map(|image| self.picker.new_resize_protocol(image));
        }
        let Some(protocol) = &mut self.protocol else {
            let label = if self.loading {
                "Loading cover…"
            } else {
                "Cover unavailable"
            };
            f.render_widget(
                Paragraph::new(label).fg(theme.text_secondary),
                design::padded(area, 1, 0),
            );
            return;
        };
        let resize = Resize::Scale(Some(FilterType::Lanczos3));
        let fitted = protocol.size_for(resize.clone(), area);
        let artwork = Rect::new(
            area.x,
            area.bottom().saturating_sub(fitted.height),
            fitted.width,
            fitted.height,
        );
        f.render_stateful_widget(StatefulImage::default().resize(resize), artwork, protocol);
        self.last_area = Some(artwork);
        if matches!(protocol.protocol_type(), StatefulProtocolType::Kitty(_)) && !artwork.is_empty()
        {
            // Match Kitty's virtual placement to the exact placeholder grid.
            let cell = &mut f.buffer_mut()[(artwork.x, artwork.y)];
            let symbol = cell.symbol().replacen(
                "a=T,U=1,",
                &format!("a=T,U=1,c={},r={},", artwork.width, artwork.height),
                1,
            );
            cell.set_symbol(&symbol);
        }
    }
}

fn artwork_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "unsupported cover URL"
    );
    anyhow::ensure!(
        url.host_str().is_some() && url.username().is_empty() && url.password().is_none(),
        "invalid cover URL"
    );
    Ok(url)
}

async fn download(client: &Client, value: &str) -> Result<DynamicImage> {
    // This client has no default bearer header. Never attach the API token here.
    let mut response = client
        .get(artwork_url(value)?)
        .timeout(Duration::from_secs(8))
        .send()
        .await?;
    anyhow::ensure!(response.status().is_success(), "cover request failed");
    anyhow::ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "cover too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_BYTES,
            "cover too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || decode(&bytes)).await?
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    Ok(if image.width() > 1024 || image.height() > 1024 {
        image.resize(1024, 1024, FilterType::Lanczos3)
    } else {
        image
    })
}

pub(super) async fn load(shared: Arc<Mutex<App>>, client: Client, request: Request) {
    let result = download(&client, &request.url).await;
    let mut app = shared.lock().await;
    app.artwork.accept(request, result);
    app.wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_detection_never_requires_a_stdin_probe_and_defaults_conservatively() {
        assert_eq!(
            terminal_protocol("xterm-kitty", "", None, true),
            ProtocolType::Kitty
        );
        assert_eq!(
            terminal_protocol("xterm-kitty", "", None, false),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            terminal_protocol("xterm-256color", "WezTerm", None, true),
            ProtocolType::Iterm2
        );
        assert_eq!(
            terminal_protocol("xterm-sixel", "", None, true),
            ProtocolType::Sixel
        );
        assert_eq!(
            terminal_protocol("tmux-256color", "tmux", None, true),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            terminal_protocol("dumb", "", Some("kitty"), false),
            ProtocolType::Kitty
        );
        assert_eq!(
            terminal_protocol("xterm-kitty", "", Some("halfblocks"), true),
            ProtocolType::Halfblocks
        );
        assert_eq!(
            terminal_protocol("xterm-kitty", "", Some("invalid"), true),
            ProtocolType::Halfblocks
        );
    }

    #[test]
    fn stale_download_cannot_restore_a_previous_tracks_cover() {
        let mut art = Artwork::new();
        art.set_url(Some("https://example.com/first.jpg"));
        let old = art.take_request().unwrap();
        art.set_url(Some("https://example.com/second.jpg"));
        art.accept(old, Ok(DynamicImage::new_rgb8(2, 2)));
        assert!(art.image.is_none() && art.loading);
        let current = art.take_request().unwrap();
        art.clear();
        art.accept(current, Ok(DynamicImage::new_rgb8(2, 2)));
        assert!(art.image.is_none() && art.url.is_none());
    }

    #[test]
    fn rejects_non_network_and_credential_bearing_artwork() {
        for value in [
            "file:///etc/passwd",
            "data:image/png,test",
            "http://remote.example/cover",
            "https://user:pass@example.com/art",
        ] {
            assert!(artwork_url(value).is_err());
        }
        assert!(artwork_url("https://cdn.example/cover.jpg").is_ok());
        assert!(decode(b"not an image").is_err());
    }

    #[tokio::test]
    async fn slow_cover_loading_does_not_lock_the_player_or_send_its_token() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/cover", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
            assert!(!request.contains("authorization") && !request.contains("secret-token"));
            ready_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            let mut bytes = Cursor::new(Vec::new());
            DynamicImage::new_rgb8(2, 2)
                .write_to(&mut bytes, ::image::ImageFormat::Png)
                .unwrap();
            let bytes = bytes.into_inner();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&bytes).await.unwrap();
        });
        let mut app = super::super::tests::app();
        app.artwork.set_url(Some(&url));
        let request = app.artwork.take_request().unwrap();
        let client = app.client.clone();
        let shared = Arc::new(Mutex::new(app));
        let loading = tokio::spawn(load(shared.clone(), client, request));
        ready_rx.await.unwrap();
        let guard = timeout(Duration::from_millis(50), shared.lock())
            .await
            .unwrap();
        assert!(guard.artwork.loading);
        drop(guard);
        finish_tx.send(()).unwrap();
        loading.await.unwrap();
        server.await.unwrap();
        let app = shared.lock().await;
        assert!(!app.artwork.loading && app.artwork.image.is_some());
    }

    #[tokio::test]
    async fn oversized_and_redirected_covers_fail_without_following_them() {
        for headers in [
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BYTES + 1),
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/cover", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                socket.write_all(headers.as_bytes()).await.unwrap();
            });
            let client = Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
            assert!(download(&client, &url).await.is_err());
        }
    }
}
