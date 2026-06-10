//! Team-Balancer — Port von `_balance_score`/`_best_split` aus
//! `cogs/deadlock_team_balancer.py` (verhaltensgleich, CPython-Referenzen).
//!
//! Beobachtung am Original (übernommen, nicht „verbessert"): Der
//! Varianz-Term (Gewicht 0,5) dominiert bei großer Rang-Spreizung — der
//! Algorithmus bevorzugt dann HOMOGENE Teams (alle Starken zusammen)
//! statt gleich starker Teams. Beispiel [66,54,48,30,12,7] →
//! [66,54,48] vs [30,12,7]. Das ist seit jeher das Live-Verhalten; eine
//! Umgewichtung wäre eine fachliche Entscheidung für später.

pub const TEAM_SIZE_CAP: usize = 6;

/// Kombinierter Score: Summen-Differenz + 2·Ø-Differenz + 0,5·(Var_A+Var_B)
/// — kleiner ist besser, leere Teams sind unendlich schlecht.
pub fn balance_score(team_a: &[i64], team_b: &[i64]) -> f64 {
    if team_a.is_empty() || team_b.is_empty() {
        return f64::INFINITY;
    }
    let sum_a: i64 = team_a.iter().sum();
    let sum_b: i64 = team_b.iter().sum();
    let avg_a = sum_a as f64 / team_a.len() as f64;
    let avg_b = sum_b as f64 / team_b.len() as f64;
    let var_a = team_a
        .iter()
        .map(|x| (*x as f64 - avg_a).powi(2))
        .sum::<f64>()
        / team_a.len() as f64;
    let var_b = team_b
        .iter()
        .map(|x| (*x as f64 - avg_b).powi(2))
        .sum::<f64>()
        / team_b.len() as f64;
    (sum_a - sum_b).abs() as f64 + (avg_a - avg_b).abs() * 2.0 + (var_a + var_b) * 0.5
}

/// Beste Aufteilung in zwei gleich große Teams (max. 6 pro Team).
/// Rückgabe: Index-Listen (Team A, Team B) in Original-Iterationsordnung.
pub fn best_split(scores: &[i64]) -> (Vec<usize>, Vec<usize>) {
    let n = scores.len();
    let team_size = (n / 2).clamp(2, TEAM_SIZE_CAP);

    let mut best: Option<(Vec<usize>, Vec<usize>)> = None;
    let mut best_score = f64::INFINITY;

    // itertools.combinations-Reihenfolge: lexikographisch aufsteigend
    let mut combo: Vec<usize> = (0..team_size).collect();
    if team_size <= n {
        loop {
            let in_a: Vec<bool> = {
                let mut mask = vec![false; n];
                for &i in &combo {
                    mask[i] = true;
                }
                mask
            };
            let rest: Vec<usize> = (0..n).filter(|i| !in_a[*i]).collect();
            if rest.len() >= team_size {
                let team_a: Vec<i64> = combo.iter().map(|&i| scores[i]).collect();
                let team_b: Vec<i64> = rest[..team_size].iter().map(|&i| scores[i]).collect();
                let score = balance_score(&team_a, &team_b);
                if score < best_score {
                    best_score = score;
                    best = Some((combo.clone(), rest[..team_size].to_vec()));
                }
            }
            // nächste Kombination
            let mut i = team_size;
            loop {
                if i == 0 {
                    break;
                }
                i -= 1;
                if combo[i] != i + n - team_size {
                    combo[i] += 1;
                    for j in (i + 1)..team_size {
                        combo[j] = combo[j - 1] + 1;
                    }
                    break;
                }
                if i == 0 {
                    combo = Vec::new();
                    break;
                }
            }
            if combo.is_empty() {
                break;
            }
        }
    }

    best.unwrap_or_else(|| {
        // Original-Fallback (kann bei n < 4 ein leeres Team B liefern —
        // bekannter Edge-Case des Originals)
        let team_a: Vec<usize> = (0..team_size.min(n)).collect();
        let team_b: Vec<usize> = (team_size..(team_size * 2).min(n)).collect();
        (team_a, team_b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_wie_python() {
        // Referenz aus CPython: score([66,30,12],[54,48,7]) = 471.777…
        assert!((balance_score(&[66, 30, 12], &[54, 48, 7]) - 471.777_777_777_777_77).abs() < 1e-9);
        assert_eq!(balance_score(&[], &[1]), f64::INFINITY);
    }

    #[test]
    fn splits_wie_python() {
        // Referenzen aus CPython (_best_split, Index-Kombination von Team A)
        assert_eq!(best_split(&[66, 54, 48, 30, 12, 7]).0, vec![0, 1, 2]);
        assert_eq!(best_split(&[60, 60, 60, 10, 10, 10]).0, vec![0, 1, 2]);
        assert_eq!(best_split(&[50, 40, 30, 20, 10]).0, vec![0, 1]);
        assert_eq!(best_split(&[36, 36, 36, 36]).0, vec![0, 1]);
        // perfekte Balance: Team B ist der Rest in Reihenfolge
        let (a, b) = best_split(&[36, 36, 36, 36]);
        assert_eq!(a.len(), b.len());
        assert_eq!(b, vec![2, 3]);
    }

    #[test]
    fn n2_fallback_wie_python() {
        // n=2: keine gültige Kombination → Original-Fallback ([0,1], [])
        let (a, b) = best_split(&[7, 72]);
        assert_eq!(a, vec![0, 1]);
        assert!(b.is_empty());
    }
}
