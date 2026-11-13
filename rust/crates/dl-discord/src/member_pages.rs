pub const MEMBER_PAGE_LIMIT: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PagerStalled;

impl std::fmt::Display for PagerStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mitgliederliste blättert nicht weiter")
    }
}

impl std::error::Error for PagerStalled {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberPager {
    after: Option<u64>,
    done: bool,
}

impl MemberPager {
    pub fn after(&self) -> Option<u64> {
        self.after
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn advance(
        &mut self,
        page_ids: impl Iterator<Item = Option<u64>>,
        page_len: usize,
    ) -> Result<(), PagerStalled> {
        if page_len == 0 {
            self.done = true;
            return Ok(());
        }
        let mut cursor = None;
        for id in page_ids {
            let id = id.ok_or(PagerStalled)?;
            cursor = Some(cursor.map_or(id, |current: u64| current.max(id)));
        }
        let cursor = cursor.ok_or(PagerStalled)?;
        if self.after.is_some_and(|previous| cursor <= previous) {
            return Err(PagerStalled);
        }
        self.after = Some(cursor);
        self.done = page_len < usize::from(MEMBER_PAGE_LIMIT);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[u64]) -> impl Iterator<Item = Option<u64>> + '_ {
        values.iter().map(|value| Some(*value))
    }

    #[test]
    fn volle_seite_blaettert_mit_dem_groessten_id_weiter() {
        let mut pager = MemberPager::default();
        let page: Vec<u64> = (1..=u64::from(MEMBER_PAGE_LIMIT)).rev().collect();
        assert_eq!(pager.advance(ids(&page), page.len()), Ok(()));
        assert_eq!(pager.after(), Some(u64::from(MEMBER_PAGE_LIMIT)));
        assert!(!pager.is_done());
    }

    #[test]
    fn kurze_seite_beendet_ohne_weitere_anfrage() {
        let mut pager = MemberPager::default();
        assert_eq!(pager.advance(ids(&[3, 9, 5]), 3), Ok(()));
        assert!(pager.is_done());
    }

    #[test]
    fn leere_seite_beendet() {
        let mut pager = MemberPager::default();
        assert_eq!(pager.advance(std::iter::empty(), 0), Ok(()));
        assert!(pager.is_done());
        assert_eq!(pager.after(), None);
    }

    #[test]
    fn stehender_cursor_oder_unlesbare_id_bricht_ab() {
        let full: Vec<u64> = (10..10 + u64::from(MEMBER_PAGE_LIMIT)).collect();
        let mut pager = MemberPager::default();
        assert_eq!(pager.advance(ids(&full), full.len()), Ok(()));
        assert_eq!(pager.advance(ids(&full), full.len()), Err(PagerStalled));
        let mut pager = MemberPager::default();
        assert_eq!(
            pager.advance([Some(4), None].into_iter(), 2),
            Err(PagerStalled)
        );
    }
}
