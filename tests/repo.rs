use dave_wang_6c2c_daily_video::{
    models::{rotation::RotationAnimal, run::RunStatus},
    repo::{advance_rotation_value, normalize_pagination, validate_run_status_transition},
};

#[test]
fn rotation_advance_follows_fixed_cycle() {
    let mut animal = RotationAnimal::Dog;
    let mut positions = Vec::new();

    for _ in 0..5 {
        let (position, next) = advance_rotation_value(animal);
        positions.push((position, next));
        animal = next;
    }

    assert_eq!(
        positions,
        vec![
            (1, RotationAnimal::Cat),
            (2, RotationAnimal::Rabbit),
            (3, RotationAnimal::Pig),
            (4, RotationAnimal::Chicken),
            (0, RotationAnimal::Dog),
        ]
    );
}

#[test]
fn run_status_transitions_allow_normal_progression_and_retry() {
    assert!(validate_run_status_transition(RunStatus::Pending, RunStatus::InProgress).is_ok());
    assert!(validate_run_status_transition(RunStatus::InProgress, RunStatus::Complete).is_ok());
    assert!(validate_run_status_transition(RunStatus::InProgress, RunStatus::Failed).is_ok());
    assert!(validate_run_status_transition(RunStatus::Failed, RunStatus::Pending).is_ok());
    assert!(validate_run_status_transition(RunStatus::Failed, RunStatus::InProgress).is_ok());
}

#[test]
fn run_status_transitions_reject_skipping_or_reopening_complete_runs() {
    assert!(validate_run_status_transition(RunStatus::Pending, RunStatus::Complete).is_err());
    assert!(validate_run_status_transition(RunStatus::Complete, RunStatus::InProgress).is_err());
    assert!(validate_run_status_transition(RunStatus::Complete, RunStatus::Failed).is_err());
}

#[test]
fn run_status_transition_is_idempotent_for_same_status() {
    assert!(validate_run_status_transition(RunStatus::Pending, RunStatus::Pending).is_ok());
    assert!(validate_run_status_transition(RunStatus::Complete, RunStatus::Complete).is_ok());
}

#[test]
fn pagination_is_clamped_to_feed_limits() {
    assert_eq!(normalize_pagination(0, -10), (1, 0));
    assert_eq!(normalize_pagination(250, 20), (100, 20));
    assert_eq!(normalize_pagination(25, 5), (25, 5));
}
