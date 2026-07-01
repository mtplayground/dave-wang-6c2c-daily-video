#![allow(dead_code)]

use std::{
    error::Error,
    fmt::{self, Display},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const ROTATION_STATE_KEY: &str = "animal_rotation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "rotation_animal")]
#[serde(rename_all = "snake_case")]
pub enum RotationAnimal {
    #[sqlx(rename = "dog")]
    Dog,
    #[sqlx(rename = "cat")]
    Cat,
    #[sqlx(rename = "rabbit")]
    Rabbit,
    #[sqlx(rename = "pig")]
    Pig,
    #[sqlx(rename = "chicken")]
    Chicken,
}

impl RotationAnimal {
    pub const CYCLE: [Self; 5] = [Self::Dog, Self::Cat, Self::Rabbit, Self::Pig, Self::Chicken];

    pub fn position(self) -> i16 {
        match self {
            Self::Dog => 0,
            Self::Cat => 1,
            Self::Rabbit => 2,
            Self::Pig => 3,
            Self::Chicken => 4,
        }
    }

    pub fn from_position(position: i16) -> Result<Self, InvalidRotationPosition> {
        match position {
            0 => Ok(Self::Dog),
            1 => Ok(Self::Cat),
            2 => Ok(Self::Rabbit),
            3 => Ok(Self::Pig),
            4 => Ok(Self::Chicken),
            _ => Err(InvalidRotationPosition { position }),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Dog => Self::Cat,
            Self::Cat => Self::Rabbit,
            Self::Rabbit => Self::Pig,
            Self::Pig => Self::Chicken,
            Self::Chicken => Self::Dog,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct RotationState {
    pub key: String,
    pub current_position: i16,
    pub current_animal: RotationAnimal,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl RotationState {
    pub fn next_animal(&self) -> RotationAnimal {
        self.current_animal.next()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRotationPosition {
    pub position: i16,
}

impl Display for InvalidRotationPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid rotation position {}: expected value from 0 through 4",
            self.position
        )
    }
}

impl Error for InvalidRotationPosition {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_order_matches_issue_scope() {
        assert_eq!(
            RotationAnimal::CYCLE,
            [
                RotationAnimal::Dog,
                RotationAnimal::Cat,
                RotationAnimal::Rabbit,
                RotationAnimal::Pig,
                RotationAnimal::Chicken
            ]
        );
    }

    #[test]
    fn next_wraps_after_chicken() {
        assert_eq!(RotationAnimal::Dog.next(), RotationAnimal::Cat);
        assert_eq!(RotationAnimal::Chicken.next(), RotationAnimal::Dog);
    }

    #[test]
    fn position_round_trips_to_animal() {
        for animal in RotationAnimal::CYCLE {
            assert_eq!(RotationAnimal::from_position(animal.position()), Ok(animal));
        }
    }

    #[test]
    fn invalid_position_is_rejected() {
        assert_eq!(
            RotationAnimal::from_position(5),
            Err(InvalidRotationPosition { position: 5 })
        );
    }
}
