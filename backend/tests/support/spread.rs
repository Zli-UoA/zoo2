//! Ids spread over the whole `users` table, whatever P is.
//!
//! `n * 37 % users` (n = 1..=P) only reaches the first 37*P ids: 12% of a 3m table at P=10,000.
//! The stride here is near the golden ratio of `users` (a Weyl sequence: every prefix of the n is
//! spread evenly) and coprime with it (so the ids of n = 1..=users are distinct).

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The stride `k` of the position `1 + n * k % users`.
pub fn stride(users: i32) -> i64 {
    let users = i64::from(users);
    let mut stride = (users as f64 * 0.618_033_988_749_895) as i64;
    while gcd(stride, users) != 1 {
        stride += 1;
    }
    stride
}

#[test]
fn the_first_n_positions_cover_the_whole_table_without_repeats() {
    for users in [10_000, 300_000, 1_000_000, 3_000_000] {
        let stride = stride(users);
        let positions: std::collections::BTreeSet<i64> = (1..=10_000_i64)
            .map(|n| 1 + n * stride % i64::from(users))
            .collect();
        assert_eq!(positions.len(), 10_000, "{users}: repeated ids");
        // Every tenth of the id range holds about a tenth of the 10,000 ids.
        for tenth in 0..10_i64 {
            let in_tenth = positions
                .iter()
                .filter(|&&p| {
                    p > tenth * i64::from(users) / 10 && p <= (tenth + 1) * i64::from(users) / 10
                })
                .count();
            assert!(
                (800..=1_200).contains(&in_tenth),
                "{users}: tenth {tenth} holds {in_tenth}"
            );
        }
    }
}
