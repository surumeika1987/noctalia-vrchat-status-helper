use anyhow::{Result, bail};
use vrchatapi::models::UserStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusUpdate {
    pub status: UserStatus,
    pub message: String,
}

impl StatusUpdate {
    pub fn parse(payload: &str) -> Result<Self> {
        let (number, message) = payload
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("payload must be <status-number>:<message>"))?;
        if number.len() != 1 {
            bail!("status number must be one digit");
        }
        let status = match number {
            "4" => UserStatus::JoinMe,
            "3" => UserStatus::Active,
            "2" => UserStatus::AskMe,
            "1" => UserStatus::Busy,
            "0" => UserStatus::Offline,
            _ => bail!("status number must be between 0 and 4"),
        };
        Ok(Self {
            status,
            message: message.to_owned(),
        })
    }

    pub fn payload(&self) -> String {
        let number = match self.status {
            UserStatus::JoinMe => 4,
            UserStatus::Active => 3,
            UserStatus::AskMe => 2,
            UserStatus::Busy => 1,
            UserStatus::Offline => 0,
        };
        format!("{number}:{}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trip_allows_empty_message() {
        let status = StatusUpdate::parse("0:").unwrap();
        assert_eq!(status.status, UserStatus::Offline);
        assert_eq!(status.message, "");
        assert_eq!(status.payload(), "0:");
    }

    #[test]
    fn message_may_contain_colons() {
        assert_eq!(StatusUpdate::parse("3:a:b").unwrap().message, "a:b");
    }
}
