use super::super::{Category, Request, SendError, Status};

#[derive(Clone, Copy, Debug)]
pub struct DisconnectRequest;

impl Request for DisconnectRequest {
    const CATEGORY: Category = Category(0x03);

    type Response = DisconnectResponse;

    fn to_bytes(&self, _buffer: &mut [u8]) -> Result<usize, SendError> {
        Ok(0)
    }

    fn from_bytes(&self, buffer: &[u8]) -> Result<Self::Response, SendError> {
        Ok(DisconnectResponse(Status::from_byte(buffer[0])?))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DisconnectResponse(pub(crate) Status);
