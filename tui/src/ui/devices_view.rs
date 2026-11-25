//! 端末と検証の画面（`D`）と、他のユーザーの検証のダイアログの描画（`ui::devices`）。文言は Web の
//! `e2ee.devices.*`、`e2ee.revocation.*`、`e2ee.verification.*`、`e2ee.sas.*`、`e2ee.qr.*`、`e2ee.authz.*` と同じ。

use qrcodegen::{QrCode, QrCodeEcc, QrSegment};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use zoo_crypto::{RevocationScope, SasPhase, UserState};

use super::app::App;
use super::devices::{
    Item, SAS_EMOJI, SasEntry, Screen, cancel_text, fingerprint, note_text, stage_text,
    user_state_text, warning_text,
};
use crate::e2ee::reauth;
use crate::e2ee::revocation::not_resigned_users;
use crate::e2ee::verification::{SasErrorCode, SaveState};

const ACCENT: Color = Color::Cyan;
const DIM: Style = Style::new().fg(Color::DarkGray);
const WIDTH: u16 = 92;

fn key(text: &str) -> Span<'static> {
    Span::styled(text.to_owned(), Style::new().fg(ACCENT))
}

fn warn(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::new().fg(Color::Yellow))
}

fn error(text: impl Into<String>) -> Line<'static> {
    Line::styled(text.into(), Style::new().fg(Color::Red))
}

fn heading(text: &str) -> Line<'static> {
    Line::styled(text.to_owned(), Style::new().add_modifier(Modifier::BOLD))
}

fn keys(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (k, label)) in pairs.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" / "));
        }
        spans.push(key(k));
        spans.push(Span::raw(format!(": {label}")));
    }
    Line::from(spans)
}

/// サーバーの日時（ISO 8601 か、PostgreSQL の形）を読む。タイムゾーンのない値は UTC とみなす。
fn parse_time(value: &str) -> Option<jiff::Timestamp> {
    let normalized = value.replacen(' ', "T", 1);
    normalized.parse::<jiff::Timestamp>().ok().or_else(|| {
        normalized
            .parse::<jiff::civil::DateTime>()
            .ok()
            .and_then(|at| at.to_zoned(jiff::tz::TimeZone::UTC).ok())
            .map(|zoned| zoned.timestamp())
    })
}

/// サーバーの日時を、端末のタイムゾーンの「年-月-日 時:分」にする。読めなければそのまま出す。
pub fn format_time(value: &str) -> String {
    match parse_time(value) {
        Some(at) => at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
        None => value.to_owned(),
    }
}

/// 30 日以上使っていない端末（20 台の上限の `stale_devices` と同じ日数）。
const STALE_DEVICE_DAYS: i64 = 30;
/// 自己署名のない（解錠していない）まま、この日数以上使っていない端末にも印を付ける。
const NOT_UNLOCKED_UNUSED_DAYS: i64 = 7;

/// 端末を消すかを考えてもらう印（Web の `deviceCleanup.ts` と同じ規則）。今の端末には付けない。使った日時が
/// なければ登録日時で数える。自己署名のある端末が 1 台もない（クロス署名鍵がまだない）ときは、署名のない
/// ことでは印を付けない。
pub fn cleanup_reasons(
    device: &crate::e2ee::revocation::DeviceView,
    now: jiff::Timestamp,
    any_signed: bool,
) -> Vec<String> {
    if device.current {
        return Vec::new();
    }
    let used = device
        .last_seen_at
        .as_deref()
        .and_then(parse_time)
        .or_else(|| parse_time(&device.created_at));
    let Some(used) = used else {
        return Vec::new();
    };
    let idle = now.as_second() - used.as_second();
    let mut reasons = Vec::new();
    if idle >= STALE_DEVICE_DAYS * 86_400 {
        reasons.push(format!("{STALE_DEVICE_DAYS} 日以上使っていない"));
    }
    if any_signed && !device.signed && idle >= NOT_UNLOCKED_UNUSED_DAYS * 86_400 {
        reasons.push(format!(
            "解錠しないまま {NOT_UNLOCKED_UNUSED_DAYS} 日以上使っていない"
        ));
    }
    reasons
}

/// 幅 `width` の枠に入れたときの行数（枠を除く）。折り返す行には 1 行足す。
/// 枠（幅 `width`）の内側の幅で、行を書記素ごとに（書記素の文字列の表示幅で）折り返す。描画と、選んだ行やスクロールの計算で同じ行を使う
/// ため、描画の側では折り返さない。返す 2 つ目は、元の各行が始まる折り返し後の行の番号。
fn wrap_lines(lines: Vec<Line<'static>>, width: u16) -> (Vec<Line<'static>>, Vec<usize>) {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut starts = Vec::with_capacity(lines.len());
    for line in lines {
        starts.push(out.len());
        if line.width() <= inner {
            out.push(line);
            continue;
        }
        let line_style = line.style;
        let mut current: Vec<Span<'static>> = Vec::new();
        let mut used = 0;
        for span in line.spans {
            let style = span.style;
            let mut text = String::new();
            for grapheme in span.content.graphemes(true) {
                let w = grapheme.width();
                if used + w > inner && used > 0 {
                    if !text.is_empty() {
                        current.push(Span::styled(std::mem::take(&mut text), style));
                    }
                    out.push(Line::from(std::mem::take(&mut current)).style(line_style));
                    used = 0;
                }
                text.push_str(grapheme);
                used += w;
            }
            if !text.is_empty() {
                current.push(Span::styled(text, style));
            }
        }
        if !current.is_empty() {
            out.push(Line::from(current).style(line_style));
        }
    }
    (out, starts)
}

/// 折り返したあとの行の数。
fn rows(lines: &[Line<'static>], width: u16) -> usize {
    wrap_lines(lines.to_vec(), width).0.len()
}

/// 枠の中に、本文と、下に固定する操作の案内（`footer`）を描く。`focus` は本文の元の行の番号と行数で、
/// その行が見えるように `scroll`（折り返し後の行の番号）を直す。
fn draw_box(
    frame: &mut Frame,
    title: &str,
    lines: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
    focus: Option<(usize, usize)>,
    scroll: &mut usize,
) {
    let width = WIDTH.min(frame.area().width);
    let (body, starts) = wrap_lines(lines, width);
    let (footer, _) = wrap_lines(footer, width);
    let height = ((body.len() + footer.len()) as u16)
        .saturating_add(2)
        .min(frame.area().height);
    let area = Rect {
        x: frame.area().width.saturating_sub(width) / 2,
        y: frame.area().height.saturating_sub(height) / 2,
        width,
        height,
    };
    let inner_height = usize::from(height.saturating_sub(2));
    let footer_height = footer.len().min(inner_height);
    let body_height = inner_height - footer_height;
    if let Some((line, count)) = focus {
        let first = starts.get(line).copied().unwrap_or(body.len());
        let end = starts.get(line + count).copied().unwrap_or(body.len());
        if first < *scroll {
            *scroll = first;
        } else if end > *scroll + body_height {
            *scroll = end.saturating_sub(body_height).min(first);
        }
    }
    *scroll = (*scroll).min(body.len().saturating_sub(body_height));
    frame.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::from(format!(" {title} ")));
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let body_area = Rect {
        height: body_height as u16,
        ..inside
    };
    let footer_area = Rect {
        y: inside.y + body_height as u16,
        height: footer_height as u16,
        ..inside
    };
    let visible: Vec<Line<'static>> = body.into_iter().skip(*scroll).take(body_height).collect();
    frame.render_widget(Paragraph::new(visible), body_area);
    frame.render_widget(
        Paragraph::new(footer.into_iter().take(footer_height).collect::<Vec<_>>()),
        footer_area,
    );
}

fn reauth_lines(
    lines: &mut Vec<Line<'static>>,
    over_ssh: bool,
    url: Option<&str>,
    port: Option<u16>,
) {
    lines.push(Line::from(
        "ブラウザーでログインし直してください（再認証）。",
    ));
    match (url, port) {
        (Some(url), Some(port)) => {
            if !over_ssh {
                lines.push(Line::from(
                    "ブラウザーが開かないときは、次の URL を開いてください。",
                ));
            }
            lines.push(Line::from(""));
            lines.push(Line::styled(url.to_owned(), Style::new().fg(ACCENT)));
            lines.push(Line::from(""));
            if over_ssh {
                for line in reauth::ssh_guidance(port) {
                    lines.push(warn(line));
                }
                lines.push(Line::from(""));
            }
            lines.push(Line::styled(
                format!("戻り先: http://127.0.0.1:{port}/callback（この端末で待っています）"),
                DIM,
            ));
        }
        _ => lines.push(Line::from("準備しています…")),
    }
}

fn recovery_key_lines(lines: &mut Vec<Line<'static>>, recovery_key: &str) {
    lines.push(Line::from(""));
    lines.push(Line::styled(
        format!("  {recovery_key}"),
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
    ));
    lines.push(Line::from(""));
}

/// AuthZ で行うこと（docs/e2ee.md の「失効の手順」の【決定】(1)〜(5)。Web の `AuthzSteps`）。
fn authz_lines(lines: &mut Vec<Line<'static>>, app: &App, before: bool) {
    lines.push(heading(if before {
        "始める前に AuthZ で行うこと"
    } else {
        "終わったら AuthZ で行うこと"
    }));
    let steps: &[&str] = if before {
        &[
            "「セキュリティ」で、この端末以外のすべてのセッションをログアウトする（ログインし直してから 5 分以内）。",
            "「サービス接続」で、zoo（Web、Android、TUI）の接続をすべて停止する。",
        ]
    } else {
        &[
            "「サービス接続」で、zoo の接続をもう一度すべて停止する（途中の本人確認でトークンが発行されるため）。",
            "最後に、この端末でもログアウトし、ログインし直す。",
        ]
    };
    for (index, step) in steps.iter().enumerate() {
        lines.push(Line::from(format!("{}. {step}", index + 1)));
    }
    if let Some((security, services)) = app.devices.authz_urls() {
        lines.push(Line::from(vec![
            Span::raw("AuthZ のセキュリティ: "),
            Span::styled(security, Style::new().fg(ACCENT)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("AuthZ のサービス接続: "),
            Span::styled(services, Style::new().fg(ACCENT)),
        ]));
    }
    lines.push(Line::styled(
        "端末の削除だけでは、ログインのセッションとトークンは残ります。発行済みのトークンは期限まで使えるので、乗っ取りの疑いが強いときは管理者にアカウントの停止も頼んでください。",
        DIM,
    ));
}

fn scope_text(scope: RevocationScope) -> &'static str {
    match scope {
        RevocationScope::DeviceOnly => "端末だけ",
        RevocationScope::Full => "パスキー、リカバリーキー、または SSSS の鍵",
    }
}

fn scope_description(scope: RevocationScope) -> &'static str {
    match scope {
        RevocationScope::DeviceOnly => {
            "自己署名鍵、ユーザー署名鍵、バックアップ鍵を作り直します。マスター鍵とリカバリーキーは変えません。"
        }
        RevocationScope::Full => {
            "マスター鍵、SSSS の鍵、リカバリーキーも作り直します。他のユーザーには、あなたを再検証してもらう必要があります。迷ったらこちらを選んでください。"
        }
    }
}

fn device_line(
    app: &App,
    device: &crate::e2ee::revocation::DeviceView,
    selected: bool,
    marker: Option<bool>,
) -> Vec<Line<'static>> {
    let devices = &app.devices;
    let style = if selected {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let mut first = Vec::new();
    if let Some(checked) = marker {
        first.push(Span::raw(if checked { "[x] " } else { "[ ] " }));
    }
    first.push(Span::styled(
        device
            .display_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "名前のない端末".into()),
        style,
    ));
    if device.current {
        first.push(Span::styled("（この端末）", Style::new().fg(Color::Green)));
    }
    if !device.signed {
        first.push(Span::styled("［未署名］", Style::new().fg(Color::Yellow)));
    }
    let any_signed = devices.overview.devices.iter().any(|d| d.signed);
    for reason in cleanup_reasons(device, jiff::Timestamp::now(), any_signed) {
        first.push(Span::styled(
            format!("［{reason}］"),
            Style::new().fg(Color::Yellow),
        ));
    }
    let mut second = format!("    登録 {}", format_time(&device.created_at));
    if let Some(last) = &device.last_seen_at {
        second.push_str(&format!(" / 最後に使った日時 {}", format_time(last)));
    }
    second.push_str(&format!(" / {}", device.device_id));
    vec![Line::from(first), Line::styled(second, DIM)]
}

/// 端末と検証の画面。
pub fn draw(frame: &mut Frame, app: &mut App) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    // 下に固定する操作の案内（本文が長くてスクロールしても見えるように）。
    let mut footer: Vec<Line<'static>> = Vec::new();
    // 見えるようにする本文の行（一覧の選んだ行）。あれば `app.devices.scroll` で表示位置を持つ。
    let mut focus: Option<(usize, usize)> = None;
    let over_ssh = app.devices.over_ssh;
    match &app.devices.screen {
        Screen::Loading => lines.push(Line::from("読んでいます…")),
        Screen::Working(message) => lines.push(Line::from(*message)),
        Screen::Overview => {
            let (overview_lines, cursor_line, help) = overview(app);
            lines = overview_lines;
            footer.push(help);
            focus = Some((cursor_line, 2));
        }
        Screen::Rename { input, .. } => {
            lines.push(Line::from("端末名を変える"));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("> "),
                Span::raw(input.clone()),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]));
            lines.push(Line::from(""));
            lines.push(keys(&[("Enter", "保存する"), ("Esc", "やめる")]));
        }
        Screen::ConfirmDelete { .. } => {
            lines.push(heading("端末を削除しますか"));
            lines.push(Line::from(
                "削除は失効ではありません。なくした端末や見覚えのない端末なら、取りやめて「失効を始める」から削除してください。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("y", "削除だけ行う"),
                ("R", "失効を始める"),
                ("その他", "やめる"),
            ]));
        }
        Screen::RevokeSelect {
            selected,
            cursor,
            scope,
            error: message,
        } => {
            lines.push(heading("端末と鍵の失効"));
            lines.push(Line::from(
                "なくした端末や見覚えのない端末を削除し、鍵を作り直します。他の自分の端末は、解錠し直すまで鍵を受け取れません。",
            ));
            if let Some(message) = message {
                lines.push(error(message.clone()));
            }
            lines.push(Line::from(""));
            lines.push(heading("削除する端末"));
            lines.push(Line::from(
                "なくした端末と見覚えのない端末を選んでください。端末名は根拠にせず、登録日時と最後に使った日時で確かめてください。",
            ));
            let others: Vec<_> = app
                .devices
                .overview
                .devices
                .iter()
                .filter(|d| !d.current)
                .collect();
            if others.is_empty() {
                lines.push(Line::styled("（ほかの端末はありません）", DIM));
            }
            for (index, device) in others.iter().enumerate() {
                let device_lines = device_line(
                    app,
                    device,
                    index == *cursor,
                    Some(selected.contains(&device.device_id)),
                );
                if index == *cursor {
                    focus = Some((lines.len(), device_lines.len()));
                }
                lines.extend(device_lines);
            }
            // 範囲と操作の案内は、一覧をスクロールしても見えるように下に固定する。
            footer.push(Line::from(""));
            footer.push(Line::from(vec![
                Span::styled(
                    "漏れた可能性があるもの: ",
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(scope_text(*scope)),
            ]));
            footer.push(Line::from(scope_description(*scope)));
            footer.push(warn(
                "すでに取得された過去のメッセージ、古いバックアップの中身、削除した端末が持つ鍵は、失効しても取り消せません。",
            ));
            footer.push(keys(&[
                ("j/k", "移動"),
                ("Space", "選ぶ"),
                ("t", "範囲を切り替える"),
                ("Enter", "次へ"),
                ("Esc", "やめる"),
            ]));
        }
        Screen::RevokeAuthz { .. } => {
            authz_lines(&mut lines, app, true);
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("y", "AuthZ でセッションとサービス接続を停止した"),
                ("Esc", "やめる"),
            ]));
        }
        Screen::RevokeUnlock {
            resume,
            input,
            error: message,
            ..
        } => {
            lines.push(Line::from(if *resume {
                resume_key_text(app.devices.overview.revocation.as_ref())
            } else {
                "始める前に、リカバリーキーで SSSS を開け直してください。"
            }));
            lines.push(Line::from(
                "リカバリーキーを入力してください（空白は無視します）。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("> "),
                Span::raw(input.as_str().to_owned()),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]));
            if let Some(message) = message {
                lines.push(Line::from(""));
                lines.push(error(message.clone()));
            }
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("Enter", "開ける"),
                ("Ctrl+U", "消す"),
                ("Esc", "やめる"),
            ]));
        }
        Screen::RevokeRecoveryKey(recovery_key) => {
            lines.push(Line::from(
                "新しいリカバリーキーです。控えてから実行してください。古いリカバリーキーとパスキーでは、失効のあと解錠できなくなります。",
            ));
            recovery_key_lines(&mut lines, recovery_key);
            lines.push(Line::from(
                "TUI はパスキーを使えないので、パスキーは失効のあと、Web か Android の設定の「暗号化」から足してください。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[("y", "控えたので次へ"), ("Esc", "やめる")]));
        }
        Screen::RevokeConfirm => {
            if let Some(state) = app.devices.session_state() {
                lines.push(heading("端末と鍵の失効"));
                lines.push(Line::from(format!("範囲: {}", scope_text(state.scope))));
                if !state.deleted_devices.is_empty() {
                    lines.push(Line::from(format!(
                        "削除した端末: {}",
                        state.deleted_devices.join(", ")
                    )));
                }
                if state.scope == RevocationScope::Full {
                    lines.push(warn(
                        "チャンネルの他のメンバーが、あなたを再検証するまで、あなたの新しい端末に鍵を配りません。",
                    ));
                }
                // 移せないバックアップの警告は、長くなる相手の一覧より先に出す。
                if !state.versions_without_key.is_empty() {
                    lines.push(warn(format!(
                        "新しい版に移せないバックアップがあります（版 {}）。この端末と SSSS に鍵がないので、その版のメッセージは復元できません。",
                        state.versions_without_key.join(", ")
                    )));
                }
                if state.stage > zoo_crypto::RevocationStage::Prepared {
                    lines.push(Line::from(format!(
                        "続きから保存します: {}",
                        stage_text(state.stage)
                    )));
                }
                let notes = not_resigned_users(state);
                if !notes.is_empty() {
                    lines.push(Line::from(""));
                    resign_note_lines(&mut lines, app, &notes);
                }
            }
            footer.push(Line::from(""));
            footer.push(Line::from(
                "実行すると、ログインし直して本人確認をしてから、鍵を作り直して保存します。",
            ));
            footer.push(keys(&[("Enter", "失効を実行"), ("Esc", "やめる")]));
        }
        Screen::RevokeWorking(stage) => lines.push(Line::from(stage_text(*stage))),
        Screen::RevokeDone(outcome) => {
            lines.push(Line::styled(
                "失効が終わりました。",
                Style::new().fg(Color::Green),
            ));
            if outcome.dropped_sessions > 0 {
                lines.push(warn(format!(
                    "古いバックアップの {} 件を移せませんでした。",
                    outcome.dropped_sessions
                )));
            }
            // AuthZ で行うことは、長くなる一覧より先に出す（端末が小さくても見えるように）。
            lines.push(Line::from(""));
            authz_lines(&mut lines, app, false);
            if !outcome.unsigned_devices.is_empty() {
                lines.push(Line::from(""));
                lines.push(heading("新しい鍵で署名されていない端末"));
                lines.push(Line::from(
                    "これらの端末は、パスキーかリカバリーキーで解錠し直すまで鍵を受け取れません。見覚えがなければ削除してください。",
                ));
                for device in &outcome.unsigned_devices {
                    lines.extend(device_line(app, device, false, None));
                }
            }
            if !outcome.not_resigned.is_empty() {
                lines.push(Line::from(""));
                resign_note_lines(&mut lines, app, &outcome.not_resigned);
            }
            footer.push(Line::from(""));
            footer.push(keys(&[("Enter", "閉じる")]));
        }
        Screen::RevokeFailed { message, retry } => {
            lines.push(error(message.clone()));
            lines.push(Line::from(""));
            lines.push(keys(&[
                (
                    "Enter",
                    match retry {
                        super::devices::RevokeRetry::Run => "再認証して続ける",
                        super::devices::RevokeRetry::Restart => "やり直す",
                    },
                ),
                ("Esc", "閉じる（進み具合はこの端末に残ります）"),
            ]));
        }
        Screen::ReverifyConfirm(preview) => {
            let name = app.store.display_name(&preview.user_id);
            lines.push(heading(&format!("{name} さんの今の鍵を信頼しますか")));
            lines.push(Line::from(format!(
                "下の指紋を、{name} さんの画面（設定の「暗号化」の「あなたの鍵の指紋」）と、チャンネルの外（対面や別の連絡手段）で読み合わせ、同じときだけ信頼してください。ログインし直して本人確認をしてから保存します。"
            )));
            lines.push(Line::from(""));
            lines.push(Line::from("今の鍵の指紋"));
            lines.push(Line::styled(
                format!("  {}", fingerprint(&preview.master_key)),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            if let Some(previous) = &preview.previous_master_key {
                lines.push(Line::from("前に信頼した鍵の指紋"));
                lines.push(Line::from(format!("  {}", fingerprint(previous))));
                if previous == &preview.master_key {
                    lines.push(Line::from("前に信頼した鍵と同じです。"));
                } else {
                    lines.push(warn(
                        "前に信頼した鍵から変わっています。相手が本当に鍵を作り直したか確かめてください。",
                    ));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "できれば「検証する」から、絵文字で検証してください。指紋を読み合わせなくても、相手と同じ鍵を見ているか確かめられます。",
            ));
            lines.push(Line::from(
                "信頼すると、あなたの署名鍵でこの鍵に署名し、あなたのすべての端末がこの鍵を信頼します。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("y", "この鍵を信頼する"),
                ("v", "絵文字で検証する"),
                ("Esc", "やめる"),
            ]));
        }
        Screen::ConfirmDeleteSlot { .. } => {
            lines.push(heading("解錠の手段を削除しますか"));
            lines.push(Line::from(
                "鍵スロットの削除は失効ではありません。すでに取得された鍵スロットや、解錠済みの端末の鍵は無効になりません。なくした・漏れたと思うときは、「失効を始める」を行ってください。",
            ));
            lines.push(Line::from(
                "削除の前に、ログインし直して本人確認をします。最後の 1 つは削除できません。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[("y", "削除する"), ("その他", "やめる")]));
        }
        Screen::SlotUnlock {
            input,
            error: message,
        } => {
            lines.push(heading("リカバリーキーを作り直す"));
            lines.push(Line::from(
                "今のリカバリーキーで SSSS を開けてから、新しいリカバリーキーを作ります。古いリカバリーキーでは解錠できなくなります（作り直しは失効ではありません）。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("> "),
                Span::raw(input.as_str().to_owned()),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]));
            if let Some(message) = message {
                lines.push(Line::from(""));
                lines.push(error(message.clone()));
            }
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("Enter", "開ける"),
                ("Ctrl+U", "消す"),
                ("Esc", "やめる"),
            ]));
        }
        Screen::SlotRecoveryKey { recovery_key, .. } => {
            lines.push(Line::from(
                "新しいリカバリーキー（紙やパスワードマネージャーに控えてください）:",
            ));
            recovery_key_lines(&mut lines, recovery_key);
            lines.push(Line::from(
                "保存すると、古いリカバリーキーでは解錠できなくなります。保存の前に、ログインし直して本人確認をします。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[("y", "控えたので保存する"), ("Esc", "やめる")]));
        }
        Screen::Reauth { url, port } => {
            reauth_lines(&mut lines, over_ssh, url.as_deref(), *port);
            lines.push(keys(&[("Esc", "再認証をやめる")]));
        }
        Screen::Message {
            text,
            error: failed,
        } => {
            lines.push(if *failed {
                error(text.clone())
            } else {
                Line::from(text.clone())
            });
            lines.push(Line::from(""));
            lines.push(keys(&[("Enter", "一覧に戻る"), ("Esc", "閉じる")]));
        }
    }
    // リカバリーキーを全部表示できない（端末が小さい）ときは、キーを出さずに広げるよう求め、先に進めない。
    if matches!(
        app.devices.screen,
        Screen::RevokeRecoveryKey(_) | Screen::SlotRecoveryKey { .. }
    ) {
        let width = WIDTH.min(frame.area().width);
        let fits =
            rows(&lines, width) + rows(&footer, width) + 2 <= usize::from(frame.area().height);
        app.devices.recovery_key_shown = fits;
        if !fits {
            lines = vec![
                error("端末が小さいため、リカバリーキーを全部表示できません。"),
                Line::from("端末を広げてください（広げるまで先に進めません）。"),
                keys(&[("Esc", "やめる")]),
            ];
            footer.clear();
        }
    }
    match focus {
        Some(_) => draw_box(
            frame,
            "端末と検証",
            lines,
            footer,
            focus,
            &mut app.devices.scroll,
        ),
        None => draw_box(frame, "端末と検証", lines, footer, None, &mut 0),
    }
}

/// 止まった失効を続けるときに使うリカバリーキーの案内。SSSS を保存したと記録してあれば、全体の失効では
/// 新しいキー。記録の前（SSSS を書いたかどうか分からない）なら、前のキーを先に試し、開けなければ新しいキー。
fn resume_key_text(pending: Option<&super::devices::PendingRevocation>) -> &'static str {
    match pending {
        Some(state) if state.scope == RevocationScope::DeviceOnly => {
            "止まった失効を続けるため、リカバリーキーで SSSS を開け直してください（端末だけの失効は、リカバリーキーを変えません）。"
        }
        Some(state) if state.stage >= zoo_crypto::RevocationStage::SecretStorageSaved => {
            "止まった失効を続けるため、リカバリーキーで SSSS を開け直してください。新しい SSSS を保存したあとなので、失効で控えた新しいリカバリーキーを使ってください。"
        }
        _ => {
            "止まった失効を続けるため、リカバリーキーで SSSS を開け直してください。新しい SSSS を保存できたかは分からないので、まず失効を始めたときのリカバリーキーを使い、開けなければ失効で控えた新しいリカバリーキーを試してください。"
        }
    }
}

/// 署名し直しとピンの更新についての注意（相手ごとの理由）。
fn resign_note_lines(
    lines: &mut Vec<Line<'static>>,
    app: &App,
    notes: &[(String, Vec<zoo_crypto::ResignNote>)],
) {
    lines.push(heading("署名し直しとピンの更新についての注意"));
    lines.push(Line::from(
        "相手ごとの理由です。「この端末の記録で置き換える」相手は、サーバーのピンをこの端末の記録で作り直します。ほかの相手は、新しい鍵で署名し直しません（理由によっては、失効のあと再検証が必要です）。",
    ));
    for (user, reasons) in notes {
        let reasons: Vec<&str> = reasons.iter().copied().map(note_text).collect();
        lines.push(Line::from(format!(
            "・{}: {}",
            app.store.display_name(user),
            reasons.join("、")
        )));
    }
}

/// 一覧の行と、選んだ行の位置。
fn overview(app: &App) -> (Vec<Line<'static>>, usize, Line<'static>) {
    let devices = &app.devices;
    let overview = &devices.overview;
    let items = devices.items();
    let selected = items.get(devices.cursor).copied();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cursor_line = 0;
    let limit = devices.device_limit();

    if let Some(revocation) = &overview.revocation {
        lines.push(warn("失効が終わっていません"));
        lines.push(Line::from(format!(
            "前に始めた失効の続きがあります（{}）。SSSS を開け直して、最後まで保存してください。",
            stage_text(revocation.stage)
        )));
        lines.push(keys(&[("R", "失効を続ける")]));
        lines.push(Line::from(""));
    } else if let Some(intent) = &overview.intent {
        lines.push(warn("失効が終わっていません"));
        lines.push(Line::from(format!(
            "前に始めた失効が、鍵を作り直す前に止まりました（削除する端末: {}）。同じ範囲と端末で始め直してください。",
            if intent.deleted_devices.is_empty() {
                "なし".to_owned()
            } else {
                intent.deleted_devices.join(", ")
            }
        )));
        lines.push(keys(&[("R", "失効を続ける")]));
        lines.push(Line::from(""));
    }
    if limit.is_some() {
        lines.push(error("端末の上限（20 台）に達しました"));
        lines.push(Line::from(
            "この端末を登録できません。使っていない端末を選んで削除してください。削除は失効ではないので、なくした端末があれば、登録できたあとに失効も行ってください。",
        ));
        lines.push(keys(&[("Space", "選ぶ"), ("X", "選んだ端末を削除する")]));
        lines.push(Line::from(""));
    }

    lines.push(heading("端末"));
    lines.push(Line::styled("暗号化の鍵を持つ端末の一覧です。", DIM));
    let stale: std::collections::BTreeSet<String> = limit
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.get("device_id").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect();
    for (index, device) in overview.devices.iter().enumerate() {
        let is_selected = selected == Some(Item::Device(index));
        if is_selected {
            cursor_line = lines.len();
        }
        let marker = (devices.device_limit().is_some() && !device.current)
            .then(|| devices.limit_selected.contains(&device.device_id));
        let mut device_lines = device_line(app, device, is_selected, marker);
        if stale.contains(&device.device_id) {
            device_lines.push(Line::styled(
                "    30 日以上使っていない端末",
                Style::new().fg(Color::Yellow),
            ));
        }
        lines.extend(device_lines);
    }
    lines.push(Line::styled(
        "端末の削除は失効ではありません。削除した端末は、持っている鍵で今のバックアップを読めます。なくした端末や見覚えのない端末は、「失効を始める」から削除してください。",
        DIM,
    ));
    lines.push(Line::from(""));

    lines.push(heading("他のユーザーの検証"));
    lines.push(Line::styled(
        "暗号化チャンネルを共有する相手の鍵の状態です。鍵が変わった相手には、再検証するまで鍵を配りません。",
        DIM,
    ));
    for warning in &overview.warnings {
        lines.push(warn(warning_text(*warning)));
    }
    if let Some(message) = &overview.users_error {
        lines.push(Line::styled(message.clone(), DIM));
    } else if overview.users.is_empty() {
        lines.push(Line::styled(
            "暗号化チャンネルを共有している相手はいません。",
            DIM,
        ));
    }
    for (index, user) in overview.users.iter().enumerate() {
        let is_selected = selected == Some(Item::User(index));
        if is_selected {
            cursor_line = lines.len();
        }
        let style = if is_selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let state_style = match user.state {
            UserState::NeedsReverification(_) | UserState::Blocked => {
                Style::new().fg(Color::Yellow)
            }
            UserState::Trusted(zoo_crypto::TrustBasis::UserSigningSignature) => {
                Style::new().fg(Color::Green)
            }
            _ => DIM,
        };
        lines.push(Line::from(vec![
            Span::styled(app.store.display_name(&user.user_id), style),
            Span::raw("  "),
            Span::styled(user_state_text(&user.state), state_style),
        ]));
    }
    if let Some(own) = &overview.own_master_key {
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "あなたの鍵の指紋: {}",
            fingerprint(own)
        )));
        lines.push(Line::styled(
            "相手があなたを再検証するときに、読み合わせる指紋です。",
            DIM,
        ));
    }
    lines.push(Line::from(""));

    lines.push(heading("解錠の手段"));
    lines.push(Line::styled(
        "AuthZ でパスキーを削除しても、ここで削除するまでは、そのパスキーで解錠できます。",
        DIM,
    ));
    for (index, slot) in overview.slots.iter().enumerate() {
        let is_selected = selected == Some(Item::Slot(index));
        if is_selected {
            cursor_line = lines.len();
        }
        let style = if is_selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let kind = if slot.kind == "passkey" {
            "パスキー"
        } else {
            "リカバリーキー"
        };
        let mut label = kind.to_owned();
        if let Some(name) = slot.label.as_ref().filter(|name| !name.is_empty()) {
            label.push_str(&format!("（{name}）"));
        }
        let mut detail = String::from("    ");
        if let Some(created) = &slot.created_at {
            detail.push_str(&format!("{} に追加 / ", format_time(created)));
        }
        match &slot.last_used_at {
            Some(used) => detail.push_str(&format!("最後に使った日：{}", format_time(used))),
            None => detail.push_str("まだ使っていません"),
        }
        lines.push(Line::styled(label, style));
        lines.push(Line::styled(detail, DIM));
    }
    lines.push(Line::styled(
        "TUI ではパスキーを使えません。パスキーは Web か Android から足してください。",
        DIM,
    ));
    lines.push(Line::from(""));
    if !overview.unlocked {
        lines.push(Line::styled(
            "失効と再検証は、暗号化を解錠した端末で行えます。",
            Style::new().fg(Color::Yellow),
        ));
    }

    let mut help: Vec<(&str, &str)> = vec![("j/k", "移動")];
    match selected {
        Some(Item::Device(index)) => {
            help.push(("n", "端末名を変える"));
            if overview.devices.get(index).is_some_and(|d| !d.current) {
                help.push(("x", "端末を削除"));
            }
        }
        Some(Item::User(index)) => {
            help.push(("v", "検証する"));
            if overview
                .users
                .get(index)
                .is_some_and(|u| matches!(u.state, UserState::NeedsReverification(_)))
            {
                help.push(("a", "再検証する"));
            }
        }
        Some(Item::Slot(_)) => help.push(("x", "削除")),
        None => {}
    }
    help.push(("K", "リカバリーキーを作り直す"));
    help.push((
        "R",
        if overview.revocation.is_some() || overview.intent.is_some() {
            "失効を続ける"
        } else {
            "失効を始める"
        },
    ));
    help.push(("r", "読み直す"));
    help.push(("Esc", "閉じる"));
    (lines, cursor_line, keys(&help))
}

/// QR コードを、上下 2 つのモジュールを 1 文字にして描く（Web と同じバイトモード、誤り訂正 M）。白地に黒で
/// 描き、周りに 4 モジュールの余白を置く。
pub fn qr_lines(bytes: &[u8]) -> Option<Vec<Line<'static>>> {
    let segment = QrSegment::make_bytes(bytes);
    let code = QrCode::encode_segments(&[segment], QrCodeEcc::Medium).ok()?;
    let border = 4;
    let size = code.size();
    let dark = |x: i32, y: i32| x >= 0 && y >= 0 && x < size && y < size && code.get_module(x, y);
    let style = Style::new().fg(Color::Black).bg(Color::White);
    let mut lines = Vec::new();
    let mut y = -border;
    while y < size + border {
        let mut row = String::new();
        for x in -border..size + border {
            row.push(match (dark(x, y), dark(x, y + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(Line::styled(row, style));
        y += 2;
    }
    Some(lines)
}

/// 他のユーザーの検証のダイアログ（どの画面にいても前面に出す）。
pub fn draw_verification(frame: &mut Frame, app: &mut App) {
    let Some(txn) = app.devices.sas_open.clone() else {
        return;
    };
    let Some(entry) = app.devices.sas_entries.get(&txn).cloned() else {
        return;
    };
    let name = app.store.display_name(&entry.view.their_user_id);
    let mut lines = verification_lines(&entry, &name, app.devices.over_ssh, true);
    // QR コードを全部表示できない（端末が小さい）ときは、欠けた QR コードを出さず、広げるよう案内する（絵文字の
    // 操作の案内は残す）。
    if let Some(bytes) = &entry.view.qr_code
        && matches!(
            entry.view.phase,
            SasPhase::Requested | SasPhase::ChooseMethod
        )
    {
        let width = WIDTH.min(frame.area().width);
        let qr_width = qr_lines(bytes)
            .and_then(|code| code.first().map(Line::width))
            .unwrap_or(usize::MAX);
        let fits = qr_width <= usize::from(width.saturating_sub(2))
            && rows(&lines, width) + 2 <= usize::from(frame.area().height);
        if !fits {
            lines = verification_lines(&entry, &name, app.devices.over_ssh, false);
        }
    }
    let title = match entry.view.phase {
        SasPhase::Requested | SasPhase::ChooseMethod if entry.view.qr_code.is_some() => {
            format!("{name} さんを検証します")
        }
        _ => "他のユーザーの検証".into(),
    };
    draw_box(frame, &title, lines, Vec::new(), None, &mut 0);
}

/// 検証のダイアログの行（文言は Web の `VerificationDialog`）。
///
/// `show_qr` が偽なら、QR コードの代わりに、端末を広げるか絵文字を選ぶよう案内する（端末が小さいとき）。
pub fn verification_lines(
    entry: &SasEntry,
    name: &str,
    over_ssh: bool,
    show_qr: bool,
) -> Vec<Line<'static>> {
    let view = &entry.view;
    let mut lines = Vec::new();
    let qr = |lines: &mut Vec<Line<'static>>| {
        if !show_qr && view.qr_code.is_some() {
            lines.push(warn(if view.can_start_sas {
                "端末が小さいため、QR コードを全部表示できません。端末を広げるか、s で絵文字を選んでください。"
            } else {
                "端末が小さいため、QR コードを全部表示できません。端末を広げてください。"
            }));
            return;
        }
        if let Some(bytes) = &view.qr_code
            && let Some(code) = qr_lines(bytes)
        {
            lines.push(Line::from(format!(
                "{name} さんの端末で、この QR コードを読み取ってもらってください。カメラが使えないときや、離れた相手とは、絵文字で検証できます。"
            )));
            lines.extend(code);
            lines.push(Line::styled(
                "TUI はカメラを使えないので、相手の QR コードは読み取れません。",
                DIM,
            ));
        }
    };
    match view.phase {
        SasPhase::Incoming => {
            lines.push(heading(&format!("{name} さんが検証を求めています")));
            lines.push(Line::from(
                "QR コードを読み取るか、互いの画面に同じ絵文字が出るかを見比べて、相手の鍵を確かめます。対面や通話など、チャンネルの外で相手と画面を見比べられるときに受け入れてください。",
            ));
            lines.push(Line::from(""));
            lines.push(keys(&[("y", "受け入れる"), ("n", "断る")]));
        }
        SasPhase::Requested => {
            lines.push(heading(&format!("{name} さんの承認を待っています")));
            lines.push(Line::from(
                "相手の端末に検証の要求を送りました。相手が受け入れると、絵文字が出ます。",
            ));
            if view.qr_code.is_some() {
                lines.push(Line::from(format!(
                    "{name} さんが要求を受け入れると、相手の端末でこの QR コードを読み取れます。"
                )));
            }
            qr(&mut lines);
            lines.push(Line::from(""));
            let mut help = Vec::new();
            if view.can_start_sas {
                help.push(("s", "絵文字で検証する"));
            }
            help.push(("Esc", "中止する"));
            lines.push(keys(&help));
        }
        SasPhase::ChooseMethod => {
            qr(&mut lines);
            lines.push(Line::from(""));
            let mut help = Vec::new();
            if view.can_start_sas {
                help.push(("s", "絵文字で検証する"));
            }
            help.push(("Esc", "中止する"));
            lines.push(keys(&help));
        }
        SasPhase::Exchanging => {
            lines.push(Line::from("鍵を交換しています…"));
            lines.push(keys(&[("Esc", "中止する")]));
        }
        SasPhase::Compare => {
            lines.push(heading(&format!("{name} さんと絵文字を見比べてください")));
            lines.push(Line::from(
                "相手の画面に、同じ絵文字が同じ順に並んでいるかを確かめてください。「一致する」を押すと、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。",
            ));
            lines.push(Line::from(""));
            if let Some(emoji) = view.emoji {
                let items: Vec<(&str, &str)> = emoji
                    .iter()
                    .filter_map(|&index| SAS_EMOJI.get(usize::from(index)).copied())
                    .collect();
                lines.push(Line::from(
                    items
                        .iter()
                        .map(|(emoji, _)| format!(" {emoji} "))
                        .collect::<Vec<_>>()
                        .join("  "),
                ));
                lines.push(Line::from(
                    items
                        .iter()
                        .map(|(_, label)| *label)
                        .collect::<Vec<_>>()
                        .join(" / "),
                ));
            }
            lines.push(Line::from(""));
            lines.push(keys(&[
                ("y", "一致する"),
                ("n", "一致しない"),
                ("Esc", "中止する"),
            ]));
        }
        SasPhase::Confirmed | SasPhase::QrScanned => {
            lines.push(Line::from(if view.phase == SasPhase::QrScanned {
                "相手の画面で、読み取れたことを確かめてもらっています…"
            } else {
                "相手の確認を待っています…"
            }));
            reauth_hint(&mut lines, entry, over_ssh);
            lines.push(keys(&[("Esc", "中止する")]));
        }
        SasPhase::QrConfirm => {
            lines.push(heading(&format!(
                "{name} さんが QR コードを読み取りましたか？"
            )));
            lines.push(Line::from(format!(
                "{name} さんの端末で、目の前でこの QR コードを読み取ったときだけ「読み取った」を押してください。押すと、ログインし直して本人確認をしてから、相手の鍵を検証済みとして保存します。"
            )));
            lines.push(Line::from(""));
            lines.push(keys(&[("y", "読み取った"), ("n", "読み取っていない")]));
        }
        SasPhase::Done => match entry.save {
            SaveState::Saved => {
                lines.push(Line::styled(
                    format!("{name} さんを検証しました"),
                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::from(format!(
                        "{name} さんの鍵を検証済みとして保存しました。この鍵が変わったら、再検証するまで鍵を配りません。"
                    )));
                lines.push(Line::from(""));
                lines.push(keys(&[("Enter", "閉じる")]));
            }
            SaveState::Saving | SaveState::Idle => {
                lines.push(Line::from("検証を保存しています…"));
                reauth_hint(&mut lines, entry, over_ssh);
            }
            SaveState::Login => {
                lines.push(Line::from(
                    "検証済みとして保存するには、ログインし直して本人確認をしてください。",
                ));
                lines.push(keys(&[("Enter", "ログインして保存"), ("Esc", "閉じる")]));
            }
            SaveState::Reauth => {
                lines.push(error(
                        "本人確認ができなかったため、検証を保存できませんでした。ログインし直して保存してください。",
                    ));
                lines.push(keys(&[("Enter", "保存し直す"), ("Esc", "閉じる")]));
            }
            SaveState::Failed => {
                lines.push(error("検証を保存できませんでした。"));
                lines.push(keys(&[("Enter", "保存し直す"), ("Esc", "閉じる")]));
            }
        },
        SasPhase::Cancelled => {
            lines.push(heading("検証を中止しました"));
            lines.push(Line::from(cancel_text(view)));
            lines.push(Line::from(""));
            lines.push(keys(&[("Enter", "閉じる")]));
        }
    }
    if let Some(error_code) = entry.error {
        lines.push(Line::from(""));
        lines.push(error(match error_code {
            SasErrorCode::NeedsUnlock => {
                "この端末は解錠していないため、検証できません。設定の「暗号化」で解錠してください。"
            }
            SasErrorCode::SendFailed => {
                "相手に送れませんでした。通信を確かめてから、やり直してください。"
            }
            SasErrorCode::ReceiveFailed => {
                "相手からの応答を処理できませんでした。自動で試し直します。通信を確かめてください。"
            }
            SasErrorCode::Failed => "検証を進められませんでした。",
        }));
    }
    lines
}

fn reauth_hint(lines: &mut Vec<Line<'static>>, entry: &SasEntry, over_ssh: bool) {
    if let Some((url, port)) = &entry.reauth {
        lines.push(Line::from(""));
        reauth_lines(lines, over_ssh, Some(url), Some(*port));
        lines.push(Line::from(""));
    }
}

#[cfg(test)]
mod tests {
    use zoo_crypto::SasPhase;

    use super::*;
    use crate::e2ee::verification::SasView;

    fn entry(phase: SasPhase) -> SasEntry {
        SasEntry {
            view: SasView {
                transaction_id: "t".into(),
                their_user_id: "u".into(),
                their_device_id: None,
                we_requested: true,
                started_at_ms: 0,
                phase,
                emoji: Some([0, 1, 2, 3, 4, 5, 63]),
                cancellation: None,
                method: None,
                qr_code: Some(vec![0x4d; 90]),
                can_start_sas: true,
            },
            busy: false,
            save: SaveState::Idle,
            error: None,
            reauth: None,
        }
    }

    fn text(lines: &[Line<'_>]) -> String {
        lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn compare_shows_the_emoji_and_their_names() {
        let lines = verification_lines(&entry(SasPhase::Compare), "ボブ", false, true);
        let text = text(&lines);
        assert!(text.contains("ボブ さんと絵文字を見比べてください"));
        assert!(text.contains("🐶"));
        assert!(text.contains("犬 / 猫 / ライオン / 馬 / ユニコーン / 豚 / 画びょう"));
    }

    #[test]
    fn choosing_a_method_shows_the_qr_code_but_never_scans() {
        let lines = verification_lines(&entry(SasPhase::ChooseMethod), "ボブ", false, true);
        let text = text(&lines);
        assert!(text.contains("この QR コードを読み取ってもらってください"));
        assert!(text.contains("相手の QR コードは読み取れません"));
        assert!(text.contains("絵文字で検証する"));
        assert!(!text.contains("相手の QR を読み取る"));
    }

    #[test]
    fn a_small_terminal_gets_guidance_instead_of_a_cut_qr_code() {
        let lines = verification_lines(&entry(SasPhase::ChooseMethod), "ボブ", false, false);
        let text = text(&lines);
        assert!(text.contains("端末を広げるか、s で絵文字を選んでください"));
        assert!(text.contains("s: 絵文字で検証する"));
        assert!(!text.contains('█'));
    }

    #[test]
    fn wrapping_tracks_where_each_line_starts() {
        // 内側の幅は 8。日本語は 1 文字が 2 桁なので、4 文字で折り返す。
        let lines = vec![
            Line::from("あいうえおかきく"),
            Line::from("短い"),
            Line::from("abcdefghij"),
        ];
        let (wrapped, starts) = wrap_lines(lines, 10);
        assert_eq!(starts, vec![0, 2, 3]);
        assert_eq!(wrapped.len(), 5);
        assert!(wrapped.iter().all(|line| line.width() <= 8));
        assert_eq!(wrapped[1].to_string(), "おかきく");
        assert_eq!(wrapped[4].to_string(), "ij");
    }

    #[test]
    fn wrapping_measures_whole_graphemes() {
        // `❤` は文字ごとの幅の合計が 1 だが、表示幅は 2（異体字セレクター付き）。書記素の幅で折り返す。
        let source = "abcdef\u{2764}\u{fe0f}z";
        let (wrapped, _) = wrap_lines(vec![Line::from(source)], 10);
        assert!(wrapped.iter().all(|line| line.width() <= 8), "{wrapped:?}");
        let joined: String = wrapped.iter().map(|line| line.to_string()).collect();
        assert_eq!(joined, source);
    }

    #[test]
    fn qr_codes_are_square_with_a_quiet_zone() {
        let lines = qr_lines(&[0x4d; 90]).unwrap();
        let width = lines[0].width();
        assert!(lines.iter().all(|line| line.width() == width));
        // 上下 2 つのモジュールを 1 文字にするので、行は幅のほぼ半分。
        assert_eq!(lines.len(), width.div_ceil(2));
        assert!(lines[0].to_string().trim().is_empty(), "余白がある");
    }

    #[test]
    fn idle_devices_get_cleanup_reasons_like_the_web() {
        let now: jiff::Timestamp = "2026-11-25T00:00:00Z".parse().unwrap();
        let device =
            |last: Option<&str>, signed: bool, current: bool| crate::e2ee::revocation::DeviceView {
                device_id: "D".into(),
                display_name: None,
                created_at: "2026-10-01 00:00:00+00".into(),
                last_seen_at: last.map(str::to_owned),
                current,
                signed,
            };
        assert_eq!(
            cleanup_reasons(&device(None, true, false), now, true),
            vec!["30 日以上使っていない".to_owned()]
        );
        let recent = Some("2026-11-10T00:00:00Z");
        assert_eq!(
            cleanup_reasons(&device(recent, false, false), now, true),
            vec!["解錠しないまま 7 日以上使っていない".to_owned()]
        );
        // 署名のある端末が 1 台もなければ、署名のないことでは印を付けない。今の端末にも付けない。
        assert!(cleanup_reasons(&device(recent, false, false), now, false).is_empty());
        assert!(cleanup_reasons(&device(None, false, true), now, true).is_empty());
        let today = Some("2026-11-24T00:00:00Z");
        assert!(cleanup_reasons(&device(today, false, false), now, true).is_empty());
    }

    #[test]
    fn server_times_are_shown_in_minutes() {
        let shown = format_time("2026-11-01 10:00:00+00");
        assert_eq!(shown.len(), "2026-11-01 10:00".len());
        assert_eq!(format_time("not a time"), "not a time");
    }
}
