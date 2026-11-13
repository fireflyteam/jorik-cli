//! Shared surfaces, typography and spacing.
use super::*;

impl Theme {
    pub(super) fn surface(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(28, 34, 50)
        } else {
            tint(self.bg, Color::White, 4)
        }
    }
    pub(super) fn raised(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(35, 43, 61)
        } else {
            tint(self.bg, Color::White, 8)
        }
    }
    pub(super) fn selection(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(70, 57, 102)
        } else {
            tint(self.bg, self.primary, 24)
        }
    }
    pub(super) fn text(&self) -> Color {
        Color::Rgb(236, 238, 245)
    }
    pub(super) fn success(&self) -> Color {
        Color::Rgb(118, 218, 180)
    }
    pub(super) fn warning(&self) -> Color {
        Color::Rgb(242, 196, 116)
    }
    pub(super) fn danger(&self) -> Color {
        Color::Rgb(245, 139, 153)
    }
    pub(super) fn chrome(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(22, 27, 40)
        } else {
            self.surface()
        }
    }
    pub(super) fn tracklist(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(21, 27, 41)
        } else {
            self.surface()
        }
    }
    pub(super) fn alternate_row(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(32, 41, 59)
        } else {
            self.raised()
        }
    }
    pub(super) fn table_header(&self) -> Color {
        if self.desk_palette {
            Color::Rgb(48, 60, 86)
        } else {
            self.selection()
        }
    }
    pub(super) fn artist(&self) -> Color {
        Color::Rgb(168, 218, 204)
    }
    pub(super) fn album(&self) -> Color {
        Color::Rgb(201, 194, 226)
    }
    pub(super) fn length(&self) -> Color {
        Color::Rgb(228, 203, 161)
    }
}

pub(super) fn padded(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    area.inner(ratatui::layout::Margin::new(horizontal, vertical))
}

fn tint(base: Color, accent: Color, percent: u16) -> Color {
    let (Color::Rgb(r, g, b), Color::Rgb(ar, ag, ab)) = (base, accent) else {
        return base;
    };
    let mix =
        |a: u8, b: u8| ((u16::from(a) * (100 - percent) + u16::from(b) * percent) / 100) as u8;
    Color::Rgb(mix(r, ar), mix(g, ag), mix(b, ab))
}

pub(super) fn panel<'a>(title: impl Into<Line<'a>>, theme: &Theme, focused: bool) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(
            Style::default()
                .fg(if focused {
                    theme.highlight
                } else {
                    theme.border
                })
                .add_modifier(if focused {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )
        .style(Style::default().bg(theme.surface()).fg(theme.text()))
        .title(title)
        .title_style(
            Style::default()
                .fg(if focused {
                    theme.highlight
                } else {
                    theme.text_secondary
                })
                .bold(),
        )
        .padding(ratatui::widgets::Padding::horizontal(1))
}

pub(super) fn key_hint<'a>(key: &'a str, label: &'a str, theme: &Theme) -> Vec<Span<'a>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(theme.highlight)
                .bg(theme.raised())
                .bold(),
        ),
        Span::styled(format!(" {label}  "), Style::default().fg(theme.text())),
    ]
}

pub(super) fn primary_action(key: &str, label: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {key}  {label} "),
        Style::default().fg(theme.bg).bg(theme.highlight).bold(),
    ))
}

/// Clip by terminal cells, not bytes. Bound work even for pathological server text.
pub(super) fn fit_text(text: &str, width: u16) -> String {
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    for _ in 0..usize::from(width).saturating_mul(4).saturating_add(16) {
        let Some(c) = chars.next() else {
            return result;
        };
        if c.is_control() {
            continue;
        }
        result.push(c);
        if Line::from(result.as_str()).width() > usize::from(width) {
            result.pop();
            break;
        }
        if chars.peek().is_none() {
            return result;
        }
    }
    while !result.is_empty() && Line::from(result.as_str()).width() >= usize::from(width) {
        result.pop();
    }
    result.push('…');
    result
}

pub(super) fn single_line(text: &str, app: &App) -> String {
    crate::output::safe_text(text, app.token.as_deref()).replace(['\n', '\r', '\t'], " ")
}

pub(super) fn time_label(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

pub(super) fn prepare_modal(f: &mut Frame, area: Rect, theme: &Theme) {
    // Keep the underlying screen recognizable, but remove competing focus accents.
    for cell in &mut f.buffer_mut().content {
        cell.set_fg(theme.border).set_bg(theme.bg);
    }
    let shadow = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width,
        area.height,
    )
    .intersection(f.area());
    f.render_widget(Block::default().bg(Color::Rgb(4, 5, 10)), shadow);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().bg(theme.surface()), area);
}
