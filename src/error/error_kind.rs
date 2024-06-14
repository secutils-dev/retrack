/// Describes an application specific error types.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Error caused by the error on the client side.
    ClientError,
    /// Unknown error.
    Unknown,
}
