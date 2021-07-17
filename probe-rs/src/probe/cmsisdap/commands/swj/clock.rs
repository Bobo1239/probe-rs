use super::super::{Category, Request, SendError, Status};

#[derive(Debug)]
pub struct SWJClockRequest(pub(crate) u32);

impl Request for SWJClockRequest {
    const CATEGORY: Category = Category(0x11);

    type Response = SWJClockResponse;

    fn to_bytes(&self, buffer: &mut [u8]) -> Result<usize, SendError> {
        use scroll::{Pwrite, LE};

        buffer.pwrite_with(self.0, 0, LE).unwrap();
        Ok(4)
    }

    fn from_bytes(&self, buffer: &[u8]) -> Result<Self::Response, SendError> {
        Ok(SWJClockResponse(Status::from_byte(buffer[0])?))
    }
}

#[derive(Debug)]
pub(crate) struct SWJClockResponse(pub(crate) Status);
