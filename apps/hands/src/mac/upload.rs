//! Sending to the server through the system's URL loading (`NSURLSession`), so the Mac's own
//! TLS and trust settings apply and no TLS stack is built into the helper.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLResponse,
    NSURLSession,
};
use url::Url;

use crate::record::{Upload, UploadError, Uploader};

/// How long one request may take: a long call's track is large.
const TIMEOUT: Duration = Duration::from_secs(600);

/// The system's URL loading.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionUploader;

impl Uploader for SessionUploader {
    #[expect(
        unsafe_code,
        reason = "NSURLSession's upload calls are unsafe in objc2"
    )]
    fn post(&self, url: &Url, token: &str, body: Upload) -> Result<(), UploadError> {
        let failed = |text: String| UploadError { status: None, text };
        let address = NSURL::URLWithString(&NSString::from_str(url.as_str()))
            .ok_or_else(|| failed(format!("{url} is not a URL")))?;
        let request = NSMutableURLRequest::requestWithURL(&address);
        request.setHTTPMethod(&NSString::from_str("POST"));
        request.setTimeoutInterval(TIMEOUT.as_secs_f64());
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str(&format!("Bearer {token}"))),
            &NSString::from_str("Authorization"),
        );
        request.setValue_forHTTPHeaderField(
            Some(&NSString::from_str("application/octet-stream")),
            &NSString::from_str("Content-Type"),
        );
        let (done, answered) = mpsc::channel::<Result<(), UploadError>>();
        let handler = RcBlock::new(
            move |_data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
                // SAFETY: each is null or valid for the block's call.
                let (response, error) = unsafe { (response.as_ref(), error.as_ref()) };
                let result = match (error, response) {
                    (Some(error), _) => Err(UploadError {
                        status: None,
                        text: error.localizedDescription().to_string(),
                    }),
                    (None, Some(response)) => {
                        let status = response
                            .downcast_ref::<NSHTTPURLResponse>()
                            .map_or(0, NSHTTPURLResponse::statusCode);
                        if status == 200 {
                            Ok(())
                        } else {
                            Err(UploadError {
                                status: u16::try_from(status).ok(),
                                text: format!("HTTP {status}"),
                            })
                        }
                    }
                    (None, None) => Err(UploadError {
                        status: None,
                        text: "no answer".to_owned(),
                    }),
                };
                let _sent = done.send(result);
            },
        );
        let session = NSURLSession::sharedSession();
        let task = match body {
            Upload::File(path) => {
                let file = file_url(&path);
                // SAFETY: a valid request, file URL and completion handler.
                unsafe {
                    session
                        .uploadTaskWithRequest_fromFile_completionHandler(&request, &file, &handler)
                }
            }
            Upload::Bytes(bytes) => {
                let data = NSData::with_bytes(&bytes);
                // SAFETY: a valid request, data and completion handler.
                unsafe {
                    session.uploadTaskWithRequest_fromData_completionHandler(
                        &request,
                        Some(&data),
                        &handler,
                    )
                }
            }
        };
        task.resume();
        answered
            .recv_timeout(TIMEOUT + Duration::from_secs(5))
            .map_err(|_| failed("no answer in time".to_owned()))?
    }
}

/// A file's URL.
fn file_url(path: &Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}
