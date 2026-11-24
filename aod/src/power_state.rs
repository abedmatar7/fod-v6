#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    Awake,
    Asleep,
    Dozing,
    Unknown,
}

impl PowerState {
    pub fn parse_from_log_line(line: &str) -> Option<Self> {
        if line.contains("mWakefulness=Awake")
            || line.contains("mWakefulness=1")
            || line.contains("mInteractive=true")
        {
            return Some(PowerState::Awake);
        }
        if line.contains("mWakefulness=Dozing") || line.contains("mWakefulness=3") {
            return Some(PowerState::Dozing);
        }
        if line.contains("mWakefulness=Asleep")
            || line.contains("mWakefulness=0")
            || line.contains("mInteractive=false")
        {
            return Some(PowerState::Asleep);
        }
        None
    }
}
