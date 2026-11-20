//! Minimal dependency-free PDF writer (Helvetica, lines, filled rects) and
//! the landscape scene cue report (spec §14).

use crate::model::Category;
use crate::report::{cue_rows, header, overview_layout, CueRow};
use crate::state::ProjectState;

pub const LETTER_LANDSCAPE: (f32, f32) = (792.0, 612.0);

pub struct Pdf {
    pub w: f32,
    pub h: f32,
    pages: Vec<Vec<u8>>,
}

fn win_ansi(s: &str) -> Vec<u8> {
    let mut out = vec![];
    for ch in s.chars() {
        let b = match ch {
            '(' | ')' | '\\' => {
                out.push(b'\\');
                ch as u8
            }
            '•' => 0x95,
            '…' => 0x85,
            '·' => 0xB7,
            '×' => 0xD7,
            '±' => 0xB1,
            '–' => 0x96,
            '—' => 0x97,
            'é' => 0xE9,
            c if (' '..='~').contains(&c) => c as u8,
            _ => b'?',
        };
        out.push(b);
    }
    out
}

pub fn hex_rgb(c: &str) -> (f32, f32, f32) {
    let c = c.trim_start_matches('#');
    if c.len() != 6 {
        return (0.4, 0.4, 0.4);
    }
    let p = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).unwrap_or(100) as f32 / 255.0;
    (p(0), p(2), p(4))
}

impl Pdf {
    pub fn new(size: (f32, f32)) -> Self {
        Self { w: size.0, h: size.1, pages: vec![] }
    }
    pub fn new_page(&mut self) -> usize {
        self.pages.push(vec![]);
        self.pages.len() - 1
    }
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    fn cur(&mut self, p: usize) -> &mut Vec<u8> {
        &mut self.pages[p]
    }
    /// Text with top-left based y coordinate.
    pub fn text(&mut self, p: usize, x: f32, y_top: f32, size: f32, bold: bool, s: &str) {
        let y = self.h - y_top;
        let mut v = format!("BT /{} {size} Tf {x:.2} {y:.2} Td (", if bold { "F2" } else { "F1" }).into_bytes();
        v.extend(win_ansi(s));
        v.extend_from_slice(b") Tj ET\n");
        self.cur(p).extend(v);
    }
    /// Text truncated to fit `width` points (approximate Helvetica metrics).
    #[allow(clippy::too_many_arguments)]
    pub fn text_fit(&mut self, p: usize, x: f32, y_top: f32, size: f32, bold: bool, s: &str, width: f32) {
        let max = ((width / (size * 0.52)).floor() as usize).max(1);
        let t: String =
            if s.chars().count() > max { s.chars().take(max.saturating_sub(1)).collect::<String>() + "…" } else { s.to_string() };
        self.text(p, x, y_top, size, bold, &t);
    }
    pub fn color(&mut self, p: usize, rgb: (f32, f32, f32)) {
        let s = format!("{:.3} {:.3} {:.3} rg {:.3} {:.3} {:.3} RG\n", rgb.0, rgb.1, rgb.2, rgb.0, rgb.1, rgb.2);
        self.cur(p).extend(s.into_bytes());
    }
    pub fn rect(&mut self, p: usize, x: f32, y_top: f32, w: f32, h: f32, fill: bool) {
        let y = self.h - y_top - h;
        let s = format!("{x:.2} {y:.2} {w:.2} {h:.2} re {}\n", if fill { "f" } else { "S" });
        self.cur(p).extend(s.into_bytes());
    }
    pub fn line(&mut self, p: usize, x1: f32, y1: f32, x2: f32, y2: f32, width: f32) {
        let (h, s) = (self.h, format!("{width:.2} w {x1:.2} {:.2} m {x2:.2} {:.2} l S\n", self.h - y1, self.h - y2));
        let _ = h;
        self.cur(p).extend(s.into_bytes());
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets: Vec<usize> = vec![];
        let n = self.pages.len();
        // object numbering: 1 catalog, 2 pages, 3 F1, 4 F2, then per page: page obj, content obj
        let mut objs: Vec<Vec<u8>> = vec![];
        objs.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + i * 2)).collect();
        objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes());
        objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
        objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
        for (i, content) in self.pages.iter().enumerate() {
            objs.push(
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                    self.w,
                    self.h,
                    6 + i * 2
                )
                .into_bytes(),
            );
            let mut c = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            c.extend_from_slice(content);
            c.extend_from_slice(b"\nendstream");
            objs.push(c);
        }
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).into_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).into_bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").into_bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).into_bytes());
        out
    }
}

const MARGIN: f32 = 28.0;

fn page_header(pdf: &mut Pdf, p: usize, s: &ProjectState, scene_id: Option<&str>, date: &str, subtitle: &str) -> f32 {
    let h = header(s, scene_id, date);
    let w = pdf.w;
    pdf.color(p, (0.1, 0.1, 0.1));
    pdf.text(
        p,
        MARGIN,
        MARGIN + 12.0,
        15.0,
        true,
        &format!(
            "{} — Scene {} {}",
            h.film,
            h.scene,
            if h.scene_title.is_empty() { String::new() } else { format!("· {}", h.scene_title) }
        ),
    );
    pdf.text(
        p,
        MARGIN,
        MARGIN + 27.0,
        8.5,
        false,
        &format!(
            "Reel {} · Picture {} · {} fps{} · {} Hz · Range {} – {} · {} · {}",
            if h.reel.is_empty() { "-" } else { &h.reel },
            h.picture_version,
            h.frame_rate,
            if h.drop_frame { " (drop-frame)" } else { "" },
            h.sample_rate,
            h.start_tc,
            h.end_tc,
            h.date,
            subtitle
        ),
    );
    pdf.line(p, MARGIN, MARGIN + 33.0, w - MARGIN, MARGIN + 33.0, 0.6);
    MARGIN + 44.0
}

fn legend(pdf: &mut Pdf, p: usize, y: f32) -> f32 {
    let mut x = MARGIN;
    pdf.color(p, (0.2, 0.2, 0.2));
    pdf.text(p, x, y + 7.0, 7.5, true, "Legend:");
    x += 36.0;
    for l in [
        (Category::Footsteps, "#4e9a06"),
        (Category::Props, "#73d216"),
        (Category::HardFx, "#cc0000"),
        (Category::Background, "#3465a4"),
        (Category::DesignFx, "#75507b"),
        (Category::Dialogue, "#c4a000"),
        (Category::Music, "#ce5c00"),
    ] {
        pdf.color(p, hex_rgb(l.1));
        pdf.rect(p, x, y, 9.0, 8.0, true);
        pdf.color(p, (0.2, 0.2, 0.2));
        let label = l.0.display();
        pdf.text(p, x + 12.0, y + 7.0, 7.0, false, &label);
        x += 16.0 + label.len() as f32 * 3.8;
    }
    pdf.text(p, x + 6.0, y + 7.0, 7.0, false, "| point = 1-frame tick, range = bar, repeated = bar with sub-event ticks");
    y + 16.0
}

/// Build the printable landscape cue report for one scene (or all, when `None`).
pub fn cue_report(s: &ProjectState, scene_id: Option<&str>, date: &str) -> Vec<u8> {
    let mut pdf = Pdf::new(LETTER_LANDSCAPE);
    let tb = s.tb();
    let rows: Vec<CueRow> = cue_rows(s, scene_id);
    let (start, end) = match scene_id.and_then(|i| s.scenes.get(i)) {
        Some(sc) => (sc.start_sample, sc.end_sample),
        None => {
            let a = s.scenes.values().map(|x| x.start_sample).min().unwrap_or(0);
            let z = s.scenes.values().map(|x| x.end_sample).max().unwrap_or(a + s.project.sample_rate as i64);
            (a, z)
        }
    };
    // ---- overview pages: dense views paginate across sheets (max 45 s each)
    let lanes = overview_layout(s, scene_id);
    let sr = s.project.sample_rate as i64;
    let window = 45 * sr;
    let total = (end - start).max(1);
    let sheets = ((total + window - 1) / window).max(1);
    let missing: Vec<&CueRow> = rows.iter().filter(|r| r.media_missing || r.source_status == "no media").collect();
    for sh in 0..sheets {
        let p = pdf.new_page();
        let mut y = page_header(&mut pdf, p, s, scene_id, date, &format!("Overview {}/{}", sh + 1, sheets));
        y = legend(&mut pdf, p, y);
        if sh == 0 && !missing.is_empty() {
            pdf.color(p, (0.75, 0.0, 0.0));
            let codes: Vec<String> = missing.iter().map(|r| r.code.clone()).collect();
            pdf.text_fit(p, MARGIN, y + 8.0, 8.0, true, &format!("MISSING MEDIA: {}", codes.join(", ")), pdf.w - 2.0 * MARGIN);
            y += 14.0;
        }
        let a = start + sh * window;
        let z = (a + window).min(end);
        let x0 = MARGIN + 110.0;
        let x1 = pdf.w - MARGIN;
        let xs = |t: i64| x0 + (x1 - x0) * ((t - a) as f32 / (z - a).max(1) as f32);
        // ruler
        pdf.color(p, (0.3, 0.3, 0.3));
        pdf.line(p, x0, y + 12.0, x1, y + 12.0, 0.5);
        let px_per_sec = (x1 - x0) / ((z - a) as f32 / sr as f32).max(0.001);
        let step = [1i64, 2, 5, 10, 15, 30, 60, 120, 300].into_iter().find(|st| *st as f32 * px_per_sec >= 64.0).unwrap_or(600) * sr;
        let mut t = a - (a - start).rem_euclid(step);
        while t <= z {
            if t >= a {
                let x = xs(t);
                pdf.line(p, x, y + 9.0, x, y + 15.0, 0.4);
                pdf.text(p, x - 18.0, y + 7.0, 5.8, false, &tb.tc_at(t));
            }
            t += step;
        }
        y += 22.0;
        let bottom = pdf.h - MARGIN - 20.0;
        for lane in &lanes {
            let rows_h = lane.rows.len() as f32 * 13.0 + 4.0;
            if y + rows_h > bottom {
                pdf.color(p, (0.4, 0.4, 0.4));
                pdf.text(p, MARGIN, bottom + 8.0, 7.0, false, "(lanes continue in the cue table)");
                break;
            }
            pdf.color(p, (0.93, 0.93, 0.93));
            pdf.rect(p, MARGIN, y, x1 - MARGIN, rows_h, true);
            pdf.color(p, (0.15, 0.15, 0.15));
            pdf.text_fit(p, MARGIN + 3.0, y + 10.0, 7.5, true, &lane.name, 104.0);
            let col = hex_rgb(&lane.color);
            for (ri, row) in lane.rows.iter().enumerate() {
                let ry = y + 2.0 + ri as f32 * 13.0;
                for (code, cs, ce, label) in row {
                    if *ce < a || *cs > z {
                        continue;
                    }
                    let bx = xs((*cs).max(a));
                    let bw = (xs((*ce).min(z)) - bx).max(1.5);
                    pdf.color(p, col);
                    pdf.rect(p, bx, ry, bw, 11.0, true);
                    pdf.color(p, (0.05, 0.05, 0.05));
                    pdf.text_fit(p, bx + 2.0, ry + 8.5, 6.2, false, &format!("{code} {label}"), (x1 - bx).min(bw.max(90.0)));
                }
            }
            // sub-event ticks for repeated cues
            for c in s.sorted_cues(scene_id).iter().filter(|c| c.category == lane.category && !c.sub_events.is_empty()) {
                for &e in &c.sub_events {
                    if e >= a && e <= z {
                        pdf.color(p, (0.0, 0.0, 0.0));
                        let x = xs(e);
                        pdf.line(p, x, y + rows_h - 3.0, x, y + rows_h, 0.6);
                    }
                }
            }
            y += rows_h + 3.0;
        }
    }
    // ---- cue table pages
    let cols: [(&str, f32); 13] = [
        ("Cue", 50.0),
        ("Category", 62.0),
        ("Identity", 70.0),
        ("Description", 86.0),
        ("Start TC", 50.0),
        ("End TC", 50.0),
        ("Dur", 44.0),
        ("Track", 74.0),
        ("Media", 68.0),
        ("Source", 58.0),
        ("Operator", 40.0),
        ("Picture", 46.0),
        ("Comments", 38.0),
    ];
    let per_page = 30;
    let chunks: Vec<&[CueRow]> = if rows.is_empty() { vec![&rows[..]] } else { rows.chunks(per_page).collect() };
    for (ci, chunk) in chunks.iter().enumerate() {
        let p = pdf.new_page();
        let mut y = page_header(&mut pdf, p, s, scene_id, date, &format!("Cue table {}/{}", ci + 1, chunks.len()));
        let mut x = MARGIN;
        pdf.color(p, (0.85, 0.85, 0.85));
        pdf.rect(p, MARGIN, y, pdf.w - 2.0 * MARGIN, 13.0, true);
        pdf.color(p, (0.0, 0.0, 0.0));
        for (name, w) in cols {
            pdf.text(p, x + 2.0, y + 9.5, 7.5, true, name);
            x += w;
        }
        y += 15.0;
        for (i, r) in chunk.iter().enumerate() {
            if i % 2 == 1 {
                pdf.color(p, (0.96, 0.96, 0.96));
                pdf.rect(p, MARGIN, y - 1.0, pdf.w - 2.0 * MARGIN, 13.0, true);
            }
            pdf.color(p, if r.media_missing || r.source_status == "no media" { (0.75, 0.0, 0.0) } else { (0.0, 0.0, 0.0) });
            let vals = [
                r.code.clone(),
                r.category.clone(),
                r.identity.clone(),
                r.description.clone(),
                r.start_tc.clone(),
                r.end_tc.clone(),
                if r.duration_tc.is_empty() { "point".into() } else { r.duration_tc.clone() },
                r.track.clone(),
                r.media.clone(),
                r.source_status.clone(),
                r.owner.clone(),
                r.picture_version.clone(),
                r.notes.clone(),
            ];
            let mut x = MARGIN;
            for ((_, w), v) in cols.iter().zip(vals.iter()) {
                pdf.text_fit(p, x + 2.0, y + 8.5, 6.6, false, v, w - 4.0);
                x += w;
            }
            y += 13.0;
        }
        if rows.is_empty() {
            pdf.text(p, MARGIN, y + 10.0, 9.0, false, "No cues.");
        }
    }
    // ---- footers with page numbers
    let n = pdf.page_count();
    for p in 0..n {
        let (w, h) = (pdf.w, pdf.h);
        pdf.color(p, (0.35, 0.35, 0.35));
        pdf.text(
            p,
            MARGIN,
            h - 14.0,
            7.0,
            false,
            &format!(
                "VC Sound Design · timecodes at {} {} · {} Hz",
                s.project.frame_rate.label(),
                if s.project.frame_rate.drop_frame { "drop-frame" } else { "non-drop" },
                s.project.sample_rate
            ),
        );
        pdf.text(p, w - MARGIN - 50.0, h - 14.0, 7.0, false, &format!("Page {} of {n}", p + 1));
    }
    pdf.to_bytes()
}
