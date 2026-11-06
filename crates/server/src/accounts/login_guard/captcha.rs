//! Small raster challenges. Answers stay server-side; the browser receives pixels.
use base64::Engine;
use rand::{Rng, seq::SliceRandom};

pub(super) struct Generated {
    pub kind: &'static str,
    pub image: String,
    pub choices: Vec<String>,
    pub answer: String,
    pub slider_path: Option<SliderPath>,
}

#[derive(Clone, Copy, serde::Serialize)]
pub(super) struct SliderPath {
    pub start_y: f64,
    pub slope: f64,
}

impl SliderPath {
    fn top(self, x: usize) -> usize {
        (self.start_y + x as f64 * self.slope).round() as usize
    }
}

const WIDTH: usize = 280;
const HEIGHT: usize = 80;
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPRSTUVWXYZ";

fn glyph(c: u8) -> [u8; 7] {
    match c {
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'G' => [14, 17, 16, 23, 17, 17, 15],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'J' => [7, 2, 2, 2, 18, 18, 12],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'N' => [17, 25, 25, 21, 19, 19, 17],
        b'P' => [30, 17, 17, 30, 16, 16, 16],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'U' => [17, 17, 17, 17, 17, 17, 14],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'W' => [17, 17, 17, 21, 21, 27, 17],
        b'X' => [17, 17, 10, 4, 10, 17, 17],
        b'Y' => [17, 17, 10, 4, 4, 4, 4],
        b'Z' => [31, 1, 2, 4, 8, 16, 31],
        b'+' => [0, 4, 4, 31, 4, 4, 0],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        _ => [0; 7],
    }
}

fn pixels(text: &str, hole: Option<usize>, path: Option<SliderPath>) -> String {
    let mut rng = rand::thread_rng();
    let mut rgb = vec![245_u8; WIDTH * HEIGHT * 3];
    let mut set = |x: usize, y: usize, color: [u8; 3]| {
        if x < WIDTH && y < HEIGHT {
            rgb[(y * WIDTH + x) * 3..(y * WIDTH + x) * 3 + 3].copy_from_slice(&color);
        }
    };
    // Low-contrast curves stay behind the foreground, preserving legibility.
    for _ in 0..5 {
        let baseline = rng.gen_range(10.0..70.0);
        let phase = rng.gen_range(0.0..std::f64::consts::TAU);
        let period = rng.gen_range(25.0..55.0);
        let amplitude = rng.gen_range(4.0..10.0);
        let shade = rng.gen_range(180..215);
        for x in 0..WIDTH {
            let y = (baseline + (x as f64 / period + phase).sin() * amplitude) as usize;
            set(x, y, [shade, shade + 5, shade + 10]);
        }
    }
    for _ in 0..400 {
        set(
            rng.gen_range(0..WIDTH),
            rng.gen_range(0..HEIGHT),
            [170, 180, 195],
        );
    }
    if let Some(left) = hole {
        let path = path.expect("slider path");
        // Off-path outlines add visual texture without creating another valid gap.
        for _ in 0..3 {
            let x0 = rng.gen_range(28..248);
            let y0 = rng.gen_range(4..53);
            if left.abs_diff(x0) < 30 || path.top(x0).abs_diff(y0) < 14 {
                continue;
            }
            for x in x0..x0 + 24 {
                set(x, y0, [205, 212, 220]);
                set(x, y0 + 23, [205, 212, 220]);
            }
            for y in y0..y0 + 24 {
                set(x0, y, [205, 212, 220]);
                set(x0 + 23, y, [205, 212, 220]);
            }
        }
        let top = path.top(left);
        for x in left..left + 24 {
            for y in top..top + 24 {
                set(
                    x,
                    y,
                    if x == left || x == left + 23 || y == top || y == top + 23 {
                        [30, 55, 80]
                    } else {
                        [180, 195, 210]
                    },
                );
            }
        }
    } else {
        let left = (WIDTH - text.len() * 24) / 2;
        for (index, c) in text.bytes().enumerate() {
            let top = rng.gen_range(22..34);
            let slant: i32 = rng.gen_range(-2..=2);
            let offset = rng.gen_range(0..4);
            let color = [
                rng.gen_range(15..70),
                rng.gen_range(25..80),
                rng.gen_range(45..110),
            ];
            for (row, bits) in glyph(c).into_iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) != 0 {
                        for dx in 0..3 {
                            for dy in 0..3 {
                                let skew = (row as i32 - 3) * slant / 3;
                                let x = (left + index * 24 + offset + col * 3 + dx) as i32 + skew;
                                set(x as usize, top + row * 3 + dy, color);
                            }
                        }
                    }
                }
            }
        }
    }
    // 24-bit BMP rows at this fixed width are naturally aligned to four bytes.
    let length = 54 + rgb.len();
    let mut bmp = vec![0_u8; 54];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&(length as u32).to_le_bytes());
    bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&(WIDTH as u32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(HEIGHT as u32).to_le_bytes());
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    for row in rgb.chunks_exact(WIDTH * 3).rev() {
        for pixel in row.chunks_exact(3) {
            bmp.extend([pixel[2], pixel[1], pixel[0]]);
        }
    }
    format!(
        "data:image/bmp;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bmp)
    )
}

pub(super) fn generate() -> Generated {
    let mut rng = rand::thread_rng();
    let path = if rng.gen_bool(0.5) {
        SliderPath {
            start_y: rng.gen_range(8.0..16.0),
            slope: rng.gen_range(0.08..0.12),
        }
    } else {
        SliderPath {
            start_y: rng.gen_range(44.0..50.0),
            slope: rng.gen_range(-0.12..-0.08),
        }
    };
    let (kind, text, choices, answer, hole) = match rng.gen_range(0..4) {
        0 => {
            let text: String = (0..5)
                .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                .collect();
            ("image", text.clone(), vec![], text, None)
        }
        1 => {
            let a = rng.gen_range(10..70);
            let b = rng.gen_range(1..30);
            if rng.gen_bool(0.5) {
                (
                    "math",
                    format!("{a}+{b}"),
                    vec![],
                    format!("{}", a + b),
                    None,
                )
            } else {
                (
                    "math",
                    format!("{a}-{}", b.min(a)),
                    vec![],
                    format!("{}", a - b.min(a)),
                    None,
                )
            }
        }
        2 => {
            let mut chars = ALPHABET.to_vec();
            chars.shuffle(&mut rng);
            chars.truncate(8);
            let choices = chars.iter().map(|c| (*c as char).to_string()).collect();
            chars.shuffle(&mut rng);
            let text: String = chars[..3].iter().map(|c| *c as char).collect();
            ("select", text.clone(), choices, text, None)
        }
        _ => {
            let left = rng.gen_range(32..240);
            (
                "slider",
                String::new(),
                vec![],
                left.to_string(),
                Some(left),
            )
        }
    };
    Generated {
        kind,
        image: pixels(&text, hole, hole.map(|_| path)),
        choices,
        answer,
        slider_path: hole.map(|_| path),
    }
}
