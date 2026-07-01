#![allow(dead_code)]

use std::{
    error::Error,
    fmt::{self, Display},
    time::Duration,
};

use aws_credential_types::Credentials;
use aws_sdk_s3::{
    Client,
    config::{Builder as S3ConfigBuilder, RequestChecksumCalculation},
    operation::head_object::HeadObjectError,
    presigning::PresigningConfig,
    primitives::ByteStream,
};
use aws_types::region::Region;

use crate::config::ObjectStorageConfig;

const MAX_PRESIGNED_URL_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone)]
pub struct ObjectStorage {
    client: Client,
    bucket: String,
    prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub relative_key: String,
    pub storage_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    RawVideo,
    Frame,
    Glb,
    RevealClip,
    FinalMp4,
}

#[derive(Debug)]
pub enum StorageError {
    InvalidKey { key: String, reason: &'static str },
    InvalidPresignedUrlTtl(Duration),
    Upload(Box<dyn Error + Send + Sync>),
    ExistsCheck(Box<dyn Error + Send + Sync>),
    Presign(Box<dyn Error + Send + Sync>),
}

impl ObjectStorage {
    pub fn new(config: &ObjectStorageConfig) -> Self {
        let credentials = Credentials::new(
            config.access_key_id.clone(),
            config.secret_access_key.clone(),
            None,
            None,
            "object-storage-env",
        );

        let s3_config = S3ConfigBuilder::new()
            .region(Region::new(config.region.clone()))
            .credentials_provider(credentials)
            .endpoint_url(config.endpoint.clone())
            .force_path_style(config.force_path_style)
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .build();

        Self {
            client: Client::from_conf(s3_config),
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone(),
        }
    }

    pub async fn upload_artifact(
        &self,
        relative_key: impl AsRef<str>,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<StoredObject, StorageError> {
        let relative_key = normalize_relative_key(relative_key.as_ref())?;
        let storage_key = self.storage_key(&relative_key)?;
        let content_length = bytes.len() as i64;

        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&storage_key)
            .content_length(content_length)
            .body(ByteStream::from(bytes));

        if let Some(content_type) = content_type {
            request = request.content_type(content_type);
        }

        request
            .send()
            .await
            .map_err(|err| StorageError::Upload(Box::new(err)))?;

        Ok(StoredObject {
            relative_key,
            storage_key,
        })
    }

    pub async fn exists(&self, relative_key: impl AsRef<str>) -> Result<bool, StorageError> {
        let relative_key = normalize_relative_key(relative_key.as_ref())?;
        let storage_key = self.storage_key(&relative_key)?;

        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&storage_key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) if is_not_found(&err) => Ok(false),
            Err(err) => Err(StorageError::ExistsCheck(Box::new(err))),
        }
    }

    pub async fn public_url(
        &self,
        relative_key: impl AsRef<str>,
        expires_in: Duration,
    ) -> Result<String, StorageError> {
        if expires_in > MAX_PRESIGNED_URL_TTL {
            return Err(StorageError::InvalidPresignedUrlTtl(expires_in));
        }

        let relative_key = normalize_relative_key(relative_key.as_ref())?;
        let storage_key = self.storage_key(&relative_key)?;
        let presigning_config = PresigningConfig::expires_in(expires_in)
            .map_err(|err| StorageError::Presign(Box::new(err)))?;

        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&storage_key)
            .presigned(presigning_config)
            .await
            .map_err(|err| StorageError::Presign(Box::new(err)))?;

        Ok(presigned.uri().to_string())
    }

    pub fn artifact_key(
        &self,
        kind: ArtifactKind,
        run_id: impl Display,
        file_name: impl AsRef<str>,
    ) -> Result<String, StorageError> {
        let file_name = normalize_path_segment(file_name.as_ref())?;
        let key = format!("artifacts/{}/{}/{}", kind.folder(), run_id, file_name);
        normalize_relative_key(&key)
    }

    fn storage_key(&self, relative_key: &str) -> Result<String, StorageError> {
        if relative_key.starts_with(&self.prefix) {
            return Err(StorageError::InvalidKey {
                key: relative_key.to_owned(),
                reason: "key must be relative and must not include OBJECT_STORAGE_PREFIX",
            });
        }

        Ok(format!("{}{}", self.prefix, relative_key))
    }
}

impl ArtifactKind {
    fn folder(self) -> &'static str {
        match self {
            Self::RawVideo => "raw-video",
            Self::Frame => "frames",
            Self::Glb => "glb",
            Self::RevealClip => "reveal-clips",
            Self::FinalMp4 => "final-mp4",
        }
    }
}

impl Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey { key, reason } => {
                write!(formatter, "invalid storage key {key:?}: {reason}")
            }
            Self::InvalidPresignedUrlTtl(ttl) => write!(
                formatter,
                "invalid presigned URL ttl {:?}: maximum is {:?}",
                ttl, MAX_PRESIGNED_URL_TTL
            ),
            Self::Upload(_) => write!(formatter, "failed to upload object storage artifact"),
            Self::ExistsCheck(_) => write!(formatter, "failed to check object storage artifact"),
            Self::Presign(_) => write!(formatter, "failed to generate object storage URL"),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Upload(err) => Some(err.as_ref()),
            Self::ExistsCheck(err) => Some(err.as_ref()),
            Self::Presign(err) => Some(err.as_ref()),
            Self::InvalidKey { .. } | Self::InvalidPresignedUrlTtl(_) => None,
        }
    }
}

fn normalize_relative_key(key: &str) -> Result<String, StorageError> {
    let trimmed = key.trim();

    if trimmed.is_empty() {
        return Err(StorageError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not be empty",
        });
    }

    if trimmed.starts_with('/') {
        return Err(StorageError::InvalidKey {
            key: key.to_owned(),
            reason: "key must be relative",
        });
    }

    if trimmed.split('/').any(|segment| segment == "..") {
        return Err(StorageError::InvalidKey {
            key: key.to_owned(),
            reason: "key must not contain parent-directory segments",
        });
    }

    Ok(trimmed.to_owned())
}

fn normalize_path_segment(segment: &str) -> Result<String, StorageError> {
    let trimmed = segment.trim();

    if trimmed.is_empty() || trimmed.contains('/') || trimmed == "." || trimmed == ".." {
        return Err(StorageError::InvalidKey {
            key: segment.to_owned(),
            reason: "path segment must be a single non-empty file name",
        });
    }

    Ok(trimmed.to_owned())
}

fn is_not_found<Response>(err: &aws_sdk_s3::error::SdkError<HeadObjectError, Response>) -> bool {
    err.as_service_error()
        .is_some_and(HeadObjectError::is_not_found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> ObjectStorage {
        ObjectStorage::new(&ObjectStorageConfig {
            access_key_id: "access-key".to_owned(),
            secret_access_key: "secret-key".to_owned(),
            bucket: "bucket".to_owned(),
            prefix: "app_prefix/".to_owned(),
            endpoint: "https://example.com".to_owned(),
            region: "auto".to_owned(),
            force_path_style: true,
        })
    }

    #[test]
    fn storage_key_prepends_configured_prefix() {
        let storage = storage();
        let key = storage.storage_key("artifacts/raw-video/run-1/source.mp4");

        assert!(matches!(
            key.as_deref(),
            Ok("app_prefix/artifacts/raw-video/run-1/source.mp4")
        ));
    }

    #[test]
    fn storage_key_rejects_already_prefixed_keys() {
        let storage = storage();
        let key = storage.storage_key("app_prefix/artifacts/raw-video/run-1/source.mp4");

        assert!(matches!(key, Err(StorageError::InvalidKey { .. })));
    }

    #[test]
    fn artifact_key_uses_expected_folder() {
        let storage = storage();
        let key = storage.artifact_key(ArtifactKind::FinalMp4, "run-1", "final.mp4");

        assert!(matches!(
            key.as_deref(),
            Ok("artifacts/final-mp4/run-1/final.mp4")
        ));
    }

    #[test]
    fn relative_key_rejects_parent_directory_segments() {
        let key = normalize_relative_key("artifacts/../secret.mp4");

        assert!(matches!(key, Err(StorageError::InvalidKey { .. })));
    }
}
