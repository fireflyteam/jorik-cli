//! Request ownership and outcomes. No mutation is retried automatically.
use super::*;

pub(super) const QUEUE_LIMIT: usize = 20;
pub(super) const FRAME_MIN_INTERVAL: Duration = Duration::from_micros(33_334);
pub(super) const STATE_TTL: Duration = Duration::from_secs(30);
pub(super) const MAX_INPUT_BYTES: usize = 8192;
pub(super) const MAX_HOST_BYTES: usize = 2048;
pub(super) const MAX_PALETTE_BYTES: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Session {
    generation: u64,
    base: String,
    token: Option<String>,
    guild: Option<String>,
}

impl Session {
    pub fn capture(app: &App) -> Self {
        Self {
            generation: app.session_generation,
            base: app.base_url.clone(),
            token: app.token.clone(),
            guild: app.guild_id.clone(),
        }
    }

    pub fn matches(&self, app: &App) -> bool {
        self.generation == app.session_generation
            && self.base == app.base_url
            && self.token == app.token
            && self
                .guild
                .as_ref()
                .is_none_or(|guild| app.guild_id.as_ref() == Some(guild))
    }
}

#[derive(Clone, Default)]
pub(super) enum CommandKind {
    #[default]
    Other,
    Add(String),
    Repeat(String),
}

pub(super) struct PendingCommand {
    pub started: Instant,
    pub kind: CommandKind,
}

impl From<Instant> for PendingCommand {
    fn from(started: Instant) -> Self {
        Self {
            started,
            kind: CommandKind::Other,
        }
    }
}

#[derive(Clone)]
pub(super) struct QueueRequest {
    pub id: u64,
    pub offset: usize,
    pub session: Session,
    pub queue_revision: u64,
    pub playback_revision: u64,
}

impl App {
    pub(super) fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        let session = self.session_cancel.clone();
        self.spawn_background(async move {
            tokio::select! {
                biased;
                _ = session.cancelled() => {},
                _ = future => {},
            }
        });
    }

    pub(super) fn spawn_background(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let cancel = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {},
                _ = future => {},
            }
        });
    }

    pub(super) fn wake(&self) {
        self.redraw.notify_one();
    }

    pub(super) fn begin_queue_request(&mut self) -> Option<QueueRequest> {
        if self.token.is_none()
            || self.queue_request.as_ref().is_some_and(|request| {
                request.offset == self.queue_target_offset && request.session.matches(self)
            })
        {
            return None;
        }
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request = QueueRequest {
            id: self.next_request_id,
            offset: self.queue_target_offset,
            session: Session::capture(self),
            queue_revision: self.queue_revision,
            playback_revision: self.playback_revision,
        };
        self.queue_request = Some(request.clone());
        self.wake();
        Some(request)
    }

    pub(super) fn owns_queue_request(&self, request: &QueueRequest) -> bool {
        request.session.matches(self)
            && self
                .queue_request
                .as_ref()
                .is_some_and(|pending| pending.id == request.id)
    }

    // Returns true only when a newer page snapshot is needed, never to replay a mutation.
    pub(super) fn finish_queue_request(
        &mut self,
        request: &QueueRequest,
        result: Result<Value, String>,
    ) -> bool {
        if !self.owns_queue_request(request) {
            return false;
        }
        self.queue_request = None;
        self.wake();
        if self.queue_revision != request.queue_revision {
            return self.queue_target_offset != 0;
        }
        let result = result.and_then(|mut json| {
            let target = json.get("queue").unwrap_or(&json);
            if target.get("upcoming").and_then(Value::as_array).is_none()
                || target
                    .get("current")
                    .is_some_and(|value| !value.is_object() && !value.is_null())
            {
                return Err("Invalid server response: malformed queue".into());
            }
            if let (Some(response_guild), Some(guild)) = (
                json.get("guild_id")
                    .or_else(|| json.get("guildId"))
                    .and_then(Value::as_str),
                &self.guild_id,
            ) && response_guild != guild
            {
                return Err("Invalid server response: unexpected guild".into());
            }
            if json
                .get("offset")
                .and_then(Value::as_u64)
                .is_some_and(|offset| offset != request.offset as u64)
            {
                return Err("Invalid server response: unexpected queue offset".into());
            }
            json.as_object_mut()
                .ok_or("Invalid server response: expected an object")?
                .insert("offset".into(), Value::from(request.offset));
            if self.playback_revision != request.playback_revision
                && let Some(object) = json.as_object_mut()
            {
                object.remove("playback");
                object.remove("current");
                if let Some(queue) = object.get_mut("queue").and_then(Value::as_object_mut) {
                    queue.remove("playback");
                    queue.remove("current");
                }
            }
            Ok(json)
        });
        match result {
            Ok(json) => {
                let target = json.get("queue").unwrap_or(&json);
                if request.offset > self.queue_offset
                    && target
                        .get("upcoming")
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty)
                    && json
                        .get("total_upcoming")
                        .or_else(|| target.get("total"))
                        .is_none()
                {
                    self.queue_target_offset = self.queue_offset;
                    self.notify("No more upcoming tracks");
                    return false;
                }
                if request.offset != self.queue_offset {
                    self.queue_state = ListState::default();
                }
                self.parse_queue_response(&json);
                self.queue_error = None;
                if let Some(total) = self.queue_total
                    && request.offset >= total
                    && request.offset > 0
                {
                    self.queue_target_offset = total.saturating_sub(1) / QUEUE_LIMIT * QUEUE_LIMIT;
                    return true;
                }
            }
            Err(error) => {
                self.queue_target_offset = self.queue_offset;
                self.queue_error = Some(
                    if error.contains("user_not_in_voice_channel_or_guild_unknown") {
                        "Join a voice channel, then press r to retry. Check the server in Settings (3).".into()
                    } else {
                        error
                    },
                );
            }
        }
        false
    }

    pub(super) fn repeat_label(&self) -> &str {
        if self.loop_mode_known {
            &self.loop_mode
        } else {
            "Unknown"
        }
    }

    pub(super) fn pending_add_query(&self) -> Option<&str> {
        self.pending_commands
            .values()
            .map(|pending| &pending.kind)
            .chain(self.rest_commands.values())
            .find_map(|kind| match kind {
                CommandKind::Add(query) => Some(query.as_str()),
                _ => None,
            })
    }

    pub(super) fn pending_repeat(&self) -> Option<&str> {
        self.pending_commands
            .values()
            .map(|pending| &pending.kind)
            .chain(self.rest_commands.values())
            .find_map(|kind| match kind {
                CommandKind::Repeat(mode) => Some(mode.as_str()),
                _ => None,
            })
    }

    pub(super) fn finish_command(&mut self, kind: CommandKind, result: Result<(), String>) {
        if result.is_ok() {
            // A refresh started before a successful mutation is no longer authoritative.
            self.queue_revision = self.queue_revision.wrapping_add(1);
            self.queue_request = None;
        }
        match result {
            Ok(()) => match kind {
                CommandKind::Add(query) => {
                    self.add_pending = None;
                    self.retry_query = None;
                    if self.input == query {
                        self.input.clear();
                        self.input_cursor = 0;
                        self.input_error = None;
                        self.input_mode = InputMode::Normal;
                    }
                    self.notify("Music request accepted");
                }
                CommandKind::Repeat(_) => self.notify("Repeat request accepted; refreshing state"),
                CommandKind::Other => self.notify("Command accepted"),
            },
            Err(error) => {
                let error = crate::output::safe_text(&error, self.token.as_deref());
                if let CommandKind::Add(query) = kind {
                    self.add_pending = None;
                    if self.input == query && self.input_mode == InputMode::Editing {
                        self.input_error = Some(error.clone());
                    } else {
                        self.retry_query = Some(query);
                    }
                }
                self.error_message = Some(error);
            }
        }
        self.wake();
    }

    pub(super) fn parse_ws_state(&mut self, json: &Value) -> bool {
        if self.queue_target_offset == 0 {
            self.parse_queue_response(json);
            return false;
        }
        // Broadcasts describe page zero, not the page the operator is browsing.
        let mut snapshot = json.clone();
        let has_queue = snapshot
            .get("queue")
            .unwrap_or(&snapshot)
            .get("upcoming")
            .is_some();
        if let Some(object) = snapshot.as_object_mut() {
            object.remove("upcoming");
            object.remove("offset");
            if let Some(queue) = object.get_mut("queue").and_then(Value::as_object_mut) {
                queue.remove("upcoming");
            }
        }
        self.parse_queue_response(&snapshot);
        if has_queue {
            self.queue_revision = self.queue_revision.wrapping_add(1);
        }
        has_queue
    }

    pub(super) fn latest_received(&self) -> Option<Instant> {
        self.last_playback_received
            .into_iter()
            .chain(self.last_queue_received)
            .max()
    }

    pub(super) fn queue_is_stale(&self) -> bool {
        self.last_queue_received
            .is_some_and(|at| at.elapsed() >= STATE_TTL)
    }

    pub(super) fn playback_is_stale(&self) -> bool {
        self.current_track.is_some()
            && self
                .last_playback_received
                .is_some_and(|at| at.elapsed() >= STATE_TTL)
    }

    pub(super) fn selected_track(&self) -> Option<&Track> {
        if self.focus == PlayerFocus::Playback {
            self.current_track.as_ref()
        } else {
            self.queue_state
                .selected()
                .and_then(|index| self.queue.get(index))
        }
    }

    pub(super) fn frame_interval(&self) -> Duration {
        if self.content_view() == View::Main
            && self.view == View::Main
            && self.input_mode == InputMode::Normal
            && self.visualizer_visible
            && self.has_playback_state
            && !self.paused
            && self.spectrogram.is_some()
        {
            FRAME_MIN_INTERVAL
        } else {
            Duration::from_secs(1)
        }
    }
}
