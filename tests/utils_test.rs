use rustdocs_mcp_server::{
    error::{Result, ServerError},
    utils::{ensure_dir_exists, with_context},
};
use std::{fs, io::Error as IoError, io::ErrorKind};
use tempfile::tempdir;

#[test]
fn test_ensure_dir_exists_already_exists() {
    // Test with an existing directory
    let existing_dir = tempdir().expect("Failed to create temporary directory");
    let result = ensure_dir_exists(existing_dir.path());
    assert!(result.is_ok());
}

#[test]
fn test_ensure_dir_exists_creates_dir() {
    // Test with a non-existing directory that needs to be created
    let parent_dir = tempdir().expect("Failed to create temporary directory");
    let new_dir_path = parent_dir.path().join("new_subdir");
    
    // Verify directory doesn't exist yet
    assert!(!new_dir_path.exists());
    
    // Call function to create it
    let result = ensure_dir_exists(&new_dir_path);
    assert!(result.is_ok());
    
    // Verify directory was created
    assert!(new_dir_path.exists());
    assert!(new_dir_path.is_dir());
}

#[test]
fn test_ensure_dir_exists_path_is_file() {
    // Test with a path that exists but is a file, not a directory
    let temp_dir = tempdir().expect("Failed to create temporary directory");
    let file_path = temp_dir.path().join("test_file.txt");
    
    // Create a file at this path
    fs::write(&file_path, "test content").expect("Failed to write test file");
    assert!(file_path.exists());
    assert!(file_path.is_file());
    
    // Try to ensure this path as a directory, should fail
    let result = ensure_dir_exists(&file_path);
    assert!(result.is_err());
    
    // Verify the error kind is AlreadyExists
    let err = result.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert!(err.to_string().contains("Path exists but is not a directory"));
}

#[test]
fn test_with_context_ok_result() {
    // Test with an Ok result
    let original: std::result::Result<i32, IoError> = Ok(42);
    let result: Result<i32> = with_context(original, || "Failed to process number".to_string());
    
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
}

#[test]
fn test_with_context_io_error() {
    // Test with an IO error
    let original: std::result::Result<(), IoError> = Err(IoError::new(
        ErrorKind::NotFound, 
        "File not found"
    ));
    
    let result: Result<()> = with_context(original, || "Failed to read config".to_string());
    
    assert!(result.is_err());
    
    // Extract error to verify
    let err = result.unwrap_err();
    match err {
        ServerError::Io(io_err) => {
            assert_eq!(io_err.kind(), ErrorKind::NotFound);
            let error_msg = io_err.to_string();
            assert!(error_msg.contains("Failed to read config"));
            assert!(error_msg.contains("File not found"));
        },
        _ => panic!("Expected ServerError::Io, got {:?}", err),
    }
}

#[test]
fn test_with_context_other_error() {
    // Test with non-IO error that gets wrapped into Config error
    // Create a custom error that we'll convert to ServerError
    struct CustomError;
    impl From<CustomError> for ServerError {
        fn from(_: CustomError) -> Self {
            ServerError::MissingEnvVar("API_KEY".to_string())
        }
    }
    
    let original: std::result::Result<(), CustomError> = Err(CustomError);
    let result = with_context(original, || "Custom operation failed".to_string());
    
    assert!(result.is_err());
    
    // Extract error to verify
    let err = result.unwrap_err();
    match err {
        ServerError::Config(msg) => {
            assert!(msg.contains("Custom operation failed"));
        },
        _ => panic!("Expected ServerError::Config, got {:?}", err),
    }
}
