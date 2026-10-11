//! Staging cleanup retries never remove a substituted directory or user data.
use super::*;

#[test]
fn cleanup_retries_after_user_content_is_removed() -> std::io::Result<()> {
    let scratch = crate::worker::test_tempdir()?;
    let root = Dir::open_ambient_dir(scratch.path(), cap_std::ambient_authority())?;
    root.create_dir("stage")?;
    let mut stage = Staging::open(&root, "stage".into()).map_err(std::io::Error::other)?;
    stage
        .directory()
        .map_err(std::io::Error::other)?
        .write("owned", b"owned")?;
    stage
        .directory()
        .map_err(std::io::Error::other)?
        .write("user", b"user")?;
    assert_eq!(
        stage.cleanup(&root, &["owned".into()]),
        Err("cache_stage_cleanup_failed")
    );
    assert_eq!(root.read("stage/user")?, b"user");
    root.remove_file("stage/user")?;
    stage
        .cleanup(&root, &["owned".into()])
        .map_err(std::io::Error::other)?;
    assert!(!root.exists("stage"));
    Ok(())
}

#[test]
fn cleanup_refuses_a_replaced_staging_directory() -> std::io::Result<()> {
    let scratch = crate::worker::test_tempdir()?;
    let root = Dir::open_ambient_dir(scratch.path(), cap_std::ambient_authority())?;
    root.create_dir("stage")?;
    let mut stage = Staging::open(&root, "stage".into()).map_err(std::io::Error::other)?;
    stage.directory = None;
    root.rename("stage", &root, "old")?;
    root.create_dir("stage")?;
    root.write("stage/user", b"user")?;
    assert_eq!(stage.cleanup(&root, &[]), Err("cache_stage_cleanup_failed"));
    assert_eq!(root.read("stage/user")?, b"user");
    assert!(stage.directory().is_err());
    Ok(())
}
