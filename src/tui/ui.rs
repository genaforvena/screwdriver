use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Sparkline, Wrap},
    Frame,
};

pub fn render(f: &mut Frame, state: &AppState) {
    let area = f.area();

    // Outer: top bar / center / bottom bar
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    render_top_bar(f, rows[0], state);
    render_center(f, rows[1], state);
    render_bottom_bar(f, rows[2]);

    if state.show_help {
        render_help_overlay(f, area);
    }
}

fn format_time(secs: f32) -> String {
    let s = secs as u32;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn render_top_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let filename = state
        .files
        .get(state.file_idx)
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("—");

    let status = state
        .status_msg
        .as_ref()
        .filter(|(_, t)| t.elapsed().as_secs() < 3)
        .map(|(s, _)| format!("  {s}"))
        .unwrap_or_default();

    let rate_warn = if state.sample_rate_mismatch {
        "  [!] sample rate mismatch"
    } else {
        ""
    };

    let pos_str = format!(
        "{} / {}",
        format_time(state.position_secs),
        format_time(state.duration_secs)
    );

    let left = format!(
        " {} ({}/{}){}{}",
        filename,
        state.file_idx + 1,
        state.files.len(),
        rate_warn,
        status
    );

    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(pos_str, Style::default().fg(Color::Yellow)),
        Span::raw(if state.playing { "  ▶" } else { "  ⏸" }),
    ]);

    f.render_widget(Paragraph::new(line), area);
}

fn render_center(f: &mut Frame, area: Rect, state: &AppState) {
    // Left panel (waveform + params + position) | Right panel (clip list)
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(28)])
        .split(area);

    render_left_panel(f, cols[0], state);
    render_clip_panel(f, cols[1], state);
}

fn render_left_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // waveform
            Constraint::Length(3), // pitch
            Constraint::Length(3), // tempo
            Constraint::Length(3), // position bar
        ])
        .split(area);

    // Waveform sparkline
    let waveform_data: Vec<u64> = if state.waveform_rms.is_empty() {
        vec![0]
    } else {
        // Subsample to fit the available width
        let width = rows[0].width as usize;
        if width == 0 {
            vec![0]
        } else {
            let n = state.waveform_rms.len();
            (0..width)
                .map(|i| state.waveform_rms[i * n / width])
                .collect()
        }
    };

    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(" waveform "))
        .data(&waveform_data)
        .style(Style::default().fg(Color::Green));
    f.render_widget(sparkline, rows[0]);

    // Pitch gauge (range: -12 to +12 semitones)
    let pitch = state.pitch;
    let pitch_ratio = ((pitch + 12.0) / 24.0).clamp(0.0, 1.0) as f64;
    let pitch_label = format!(
        "PITCH  {:+.1} st  [ ↑/↓ step 0.5 st | Shift+↑/↓ step 2 st ]",
        pitch
    );
    let pitch_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Magenta))
        .ratio(pitch_ratio)
        .label(pitch_label);
    f.render_widget(pitch_gauge, rows[1]);

    // Tempo gauge (range: 0.25 to 2.0)
    let tempo = state.tempo;
    let tempo_ratio = ((tempo - 0.25) / 1.75).clamp(0.0, 1.0) as f64;
    let tempo_label = format!("TEMPO  {:.2}x  [ ←/→ step 0.05 ]", tempo);
    let tempo_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL))
        .gauge_style(Style::default().fg(Color::Cyan))
        .ratio(tempo_ratio)
        .label(tempo_label);
    f.render_widget(tempo_gauge, rows[2]);

    // Position bar with in/out markers
    let pos_ratio = if state.duration_secs > 0.0 {
        (state.position_secs / state.duration_secs).clamp(0.0, 1.0) as f64
    } else {
        0.0
    };

    let in_marker = state
        .in_point
        .map(|t| format!(" I:{}", format_time(t)))
        .unwrap_or_default();
    let out_marker = state
        .out_point
        .map(|t| format!(" O:{}", format_time(t)))
        .unwrap_or_default();
    let loop_flag = if state.looping { " [LOOP]" } else { "" };
    let pos_label = format!(
        "{}{}{}",
        format_time(state.position_secs),
        in_marker,
        out_marker
    );

    let pos_style = if state.looping {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Blue)
    };

    let pos_gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" position{} ", loop_flag)),
        )
        .gauge_style(pos_style)
        .ratio(pos_ratio)
        .label(pos_label);
    f.render_widget(pos_gauge, rows[3]);
}

fn render_clip_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .clips
        .iter()
        .map(|c| {
            ListItem::new(format!(
                "{:.1}s–{:.1}s {:+.1}st {:.2}x",
                c.in_point, c.out_point, c.pitch, c.tempo
            ))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" clips "))
        .style(Style::default().fg(Color::White));
    f.render_widget(list, area);
}

fn render_bottom_bar(f: &mut Frame, area: Rect) {
    let keys = [
        ("↑↓", "pitch"),
        ("←→", "tempo"),
        ("spc", "play"),
        ("i/o", "in/out"),
        ("[/]", "jump"),
        ("l", "loop"),
        ("s", "save"),
        ("n/p", "file"),
        ("1-3", "preset"),
        ("r", "reset"),
        ("?", "help"),
        ("q", "quit"),
    ];

    let spans: Vec<Span> = keys
        .iter()
        .flat_map(|(k, v)| {
            vec![
                Span::styled(
                    format!(" {k}"),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {v} "), Style::default().fg(Color::DarkGray)),
            ]
        })
        .collect();

    let line = Line::from(spans);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::TOP));
    f.render_widget(p, area);
}

fn render_help_overlay(f: &mut Frame, area: Rect) {
    // Centre a box
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);
    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(vert[1]);
    let popup = horiz[1];

    f.render_widget(Clear, popup);

    let text = vec![
        Line::from(Span::styled(
            "  screwdriver — key bindings",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from("  ↑ / ↓          Pitch +/- 0.5 semitones"),
        Line::from("  Shift+↑/↓      Pitch +/- 2.0 semitones"),
        Line::from("  ← / →          Tempo +/- 0.05×"),
        Line::from("  Space          Play / pause"),
        Line::from("  i              Set in point"),
        Line::from("  o              Set out point"),
        Line::from("  [ / ]          Jump to in / out point"),
        Line::from("  l              Toggle loop in→out"),
        Line::from("  s              Save current clip"),
        Line::from("  n / p          Next / previous file"),
        Line::from("  1 / 2 / 3      Presets (light/classic/deep screw)"),
        Line::from("  r              Reset pitch and tempo"),
        Line::from("  ?              Toggle this help"),
        Line::from("  q              Quit"),
        Line::from(""),
        Line::from(Span::styled(
            "  Press ? to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let help = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" help ")
                .style(Style::default().bg(Color::Black)),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(help, popup);
}
