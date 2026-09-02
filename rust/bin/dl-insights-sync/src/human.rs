use std::time::Duration;

use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub fn delay_ms(lo: u64, hi: u64) -> Duration {
    let hi = hi.max(lo);
    Duration::from_millis(rand::thread_rng().gen_range(lo..=hi))
}

pub async fn pause(lo: u64, hi: u64) {
    tokio::time::sleep(delay_ms(lo, hi)).await;
}

pub fn thread_pause(lo: u64, hi: u64) {
    std::thread::sleep(delay_ms(lo, hi));
}

pub fn type_delay_ms() -> u64 {
    rand::thread_rng().gen_range(38..=96)
}

pub fn aim(rect: Rect) -> (f64, f64) {
    let mut rng = rand::thread_rng();
    let nx = rng.gen_range(0.27..0.73);
    let ny = rng.gen_range(0.34..0.66);
    (rect.x + rect.w * nx, rect.y + rect.h * ny)
}

pub fn path(from: (f64, f64), to: (f64, f64)) -> Vec<(f64, f64)> {
    let mut rng = rand::thread_rng();
    let dist = (to.0 - from.0).hypot(to.1 - from.1).max(8.0);
    let steps = ((dist / rng.gen_range(11.0..23.0)).ceil() as usize).clamp(7, 32);
    let cx = (from.0 + to.0) * 0.5 + rng.gen_range(-48.0..48.0);
    let cy = (from.1 + to.1) * 0.5 + rng.gen_range(-36.0..36.0);
    (1..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let u = t * t * (3.0 - 2.0 * t);
            let x = (1.0 - u) * (1.0 - u) * from.0 + 2.0 * (1.0 - u) * u * cx + u * u * to.0;
            let y = (1.0 - u) * (1.0 - u) * from.1 + 2.0 * (1.0 - u) * u * cy + u * u * to.1;
            (x + rng.gen_range(-1.8..1.8), y + rng.gen_range(-1.8..1.8))
        })
        .collect()
}

pub fn start_cursor() -> (f64, f64) {
    let mut rng = rand::thread_rng();
    (rng.gen_range(180.0..520.0), rng.gen_range(140.0..380.0))
}

#[cfg(test)]
mod tests {
    use super::{aim, delay_ms, path, Rect};

    #[test]
    fn ziel_bleibt_im_knopf() {
        let rect = Rect {
            x: 100.0,
            y: 40.0,
            w: 80.0,
            h: 28.0,
        };
        for _ in 0..80 {
            let (x, y) = aim(rect);
            assert!(x > rect.x + 8.0 && x < rect.x + rect.w - 8.0);
            assert!(y > rect.y + 6.0 && y < rect.y + rect.h - 6.0);
        }
    }

    #[test]
    fn mausweg_ist_kein_sprung() {
        let pts = path((10.0, 10.0), (400.0, 260.0));
        assert!(pts.len() >= 7);
        let last = *pts.last().unwrap();
        assert!((last.0 - 400.0).abs() < 12.0);
        assert!((last.1 - 260.0).abs() < 12.0);
        let mut prev = (10.0, 10.0);
        for p in &pts {
            let step = (p.0 - prev.0).hypot(p.1 - prev.1);
            assert!(step < 120.0);
            prev = *p;
        }
    }

    #[test]
    fn pause_liegt_im_fenster() {
        for _ in 0..20 {
            let d = delay_ms(200, 400);
            assert!(d.as_millis() >= 200 && d.as_millis() <= 400);
        }
    }
}
