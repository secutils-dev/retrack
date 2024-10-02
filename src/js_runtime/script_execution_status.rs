/// Defines the status of the script.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScriptExecutionStatus {
    /// Script is running.
    Running = 0,
    /// The script was terminated because it hit the memory limit.
    ReachedMemoryLimit = 1,
    /// The script was terminated because it hit the time limit.
    ReachedTimeLimit = 2,
    /// The script has been successfully completed.
    ExecutionCompleted = 3,
}

impl From<usize> for ScriptExecutionStatus {
    fn from(value: usize) -> Self {
        match value {
            1 => Self::ReachedMemoryLimit,
            2 => Self::ReachedTimeLimit,
            3 => Self::ExecutionCompleted,
            _ => Self::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScriptExecutionStatus;

    #[test]
    fn conversion() {
        assert_eq!(
            ScriptExecutionStatus::ReachedMemoryLimit,
            ScriptExecutionStatus::from(1)
        );
        assert_eq!(
            ScriptExecutionStatus::ReachedTimeLimit,
            ScriptExecutionStatus::from(2)
        );
        assert_eq!(
            ScriptExecutionStatus::ExecutionCompleted,
            ScriptExecutionStatus::from(3)
        );

        assert_eq!(
            ScriptExecutionStatus::Running,
            ScriptExecutionStatus::from(0)
        );
        assert_eq!(
            ScriptExecutionStatus::Running,
            ScriptExecutionStatus::from(100500)
        );
    }
}
