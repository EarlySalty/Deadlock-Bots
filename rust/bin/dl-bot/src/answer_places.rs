use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, PoisonError,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerPlace {
    Requester,
    Channel(u64),
}

impl AnswerPlace {
    pub fn for_message(guild_id: Option<u64>, channel_id: u64) -> Self {
        match guild_id {
            None => Self::Requester,
            Some(_) => Self::Channel(channel_id),
        }
    }
}

type Active = HashMap<u64, Vec<(u64, AnswerPlace)>>;

#[derive(Default)]
pub struct AnswerPlaces {
    next: AtomicU64,
    active: Mutex<Active>,
}

pub struct AnswerPlaceGuard {
    places: Arc<AnswerPlaces>,
    user_id: u64,
    token: u64,
}

impl AnswerPlaces {
    pub fn enter(self: &Arc<Self>, user_id: u64, place: AnswerPlace) -> AnswerPlaceGuard {
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        self.with_active(|active| active.entry(user_id).or_default().push((token, place)));
        AnswerPlaceGuard {
            places: Arc::clone(self),
            user_id,
            token,
        }
    }

    pub fn active(&self, user_id: u64) -> Vec<AnswerPlace> {
        self.with_active(|active| {
            active
                .get(&user_id)
                .map(|entries| entries.iter().map(|(_, place)| *place).collect())
                .unwrap_or_default()
        })
    }

    fn with_active<T>(&self, apply: impl FnOnce(&mut Active) -> T) -> T {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        apply(&mut active)
    }
}

impl Drop for AnswerPlaceGuard {
    fn drop(&mut self) {
        self.places.with_active(|active| {
            if let Some(entries) = active.get_mut(&self.user_id) {
                entries.retain(|(token, _)| *token != self.token);
                if entries.is_empty() {
                    active.remove(&self.user_id);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orte_gelten_nur_solange_die_anfrage_laeuft() {
        let places = Arc::new(AnswerPlaces::default());
        let public = places.enter(7, AnswerPlace::Channel(20));
        let direct = places.enter(7, AnswerPlace::for_message(None, 99));
        assert_eq!(
            places.active(7),
            [AnswerPlace::Channel(20), AnswerPlace::Requester]
        );
        assert!(places.active(8).is_empty());
        drop(public);
        assert_eq!(places.active(7), [AnswerPlace::Requester]);
        drop(direct);
        assert!(places.active(7).is_empty());
    }
}
