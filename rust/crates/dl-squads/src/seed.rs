use std::collections::HashMap;

use crate::model::SeedRoster;
use crate::store::{
    add_team_member, create_match, create_team, set_participant_status, upsert_participant_by_name,
    SquadErr,
};
use dl_db::Db;

const STATUS_WAITLIST: &str = "waitlist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedImportSummary {
    pub players: usize,
    pub pool_unassigned: usize,
    pub teams: usize,
    pub matches: usize,
}

pub async fn import_seed_roster_json(db: &Db, json: &str) -> Result<SeedImportSummary, SquadErr> {
    let roster: SeedRoster = serde_json::from_str(json)?;
    import_seed_roster(db, &roster).await
}

pub async fn import_seed_roster(
    db: &Db,
    roster: &SeedRoster,
) -> Result<SeedImportSummary, SquadErr> {
    let mut participant_ids = HashMap::new();
    let mut raw_names_by_trimmed = HashMap::new();
    for player in &roster.players {
        let name = register_seed_participant_name(&mut raw_names_by_trimmed, &player.name)?;
        let participant_id = upsert_participant_by_name(db, player).await?;
        participant_ids.insert(name, participant_id);
    }

    for player in &roster.pool_unassigned {
        let name = register_seed_participant_name(&mut raw_names_by_trimmed, &player.name)?;
        let participant_id = upsert_participant_by_name(db, player).await?;
        set_participant_status(db, participant_id, STATUS_WAITLIST).await?;
        participant_ids.insert(name, participant_id);
    }

    let mut team_ids = HashMap::new();
    for team in &roster.teams {
        let team_id = create_team(
            db,
            &team.name,
            team.coach.as_deref(),
            team.discord_role_id,
            team.discord_channel_id,
        )
        .await?;
        team_ids.insert(team.name.clone(), team_id);

        for member in &team.members {
            let member_name = normalize_seed_name(&member.name)?;
            let participant_id = participant_ids
                .get(&member_name)
                .copied()
                .ok_or_else(|| SquadErr::MissingSeedParticipant(member_name.clone()))?;
            add_team_member(
                db,
                team_id,
                participant_id,
                member.role.as_deref(),
                member.is_captain,
                member.is_bench,
            )
            .await?;
        }
    }

    for seed_match in &roster.matches {
        let team_a_id = team_ids
            .get(&seed_match.team_a)
            .copied()
            .ok_or_else(|| SquadErr::MissingSeedTeam(seed_match.team_a.clone()))?;
        let team_b_id = team_ids
            .get(&seed_match.team_b)
            .copied()
            .ok_or_else(|| SquadErr::MissingSeedTeam(seed_match.team_b.clone()))?;
        create_match(
            db,
            team_a_id,
            team_b_id,
            seed_match.when_text.as_deref(),
            seed_match.scheduled_at.as_deref(),
        )
        .await?;
    }

    Ok(SeedImportSummary {
        players: roster.players.len(),
        pool_unassigned: roster.pool_unassigned.len(),
        teams: roster.teams.len(),
        matches: roster.matches.len(),
    })
}

fn register_seed_participant_name(
    raw_names_by_trimmed: &mut HashMap<String, String>,
    raw_name: &str,
) -> Result<String, SquadErr> {
    let trimmed = normalize_seed_name(raw_name)?;
    if let Some(first_raw_name) = raw_names_by_trimmed.get(&trimmed) {
        if first_raw_name != raw_name {
            return Err(SquadErr::DuplicateSeedParticipantName {
                trimmed,
                first: first_raw_name.clone(),
                second: raw_name.to_string(),
            });
        }
    } else {
        raw_names_by_trimmed.insert(trimmed.clone(), raw_name.to_string());
    }
    Ok(trimmed)
}

fn normalize_seed_name(raw_name: &str) -> Result<String, SquadErr> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() {
        Err(SquadErr::InvalidParticipantName(raw_name.to_string()))
    } else {
        Ok(trimmed.to_string())
    }
}
