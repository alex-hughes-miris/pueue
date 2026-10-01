use pueue_lib::{GroupStatus, Task, message::TaskSelection};
use rstest::rstest;

use crate::{helper::*, internal_prelude::*};

/// The daemon should start in the same state as before shutdown, if no tasks are queued.
/// This function tests for the running state.
#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn test_start_running(#[case] compress: bool) -> Result<()> {
    let (mut settings, tempdir) = daemon_base_setup()?;
    settings.daemon.compress_state_file = compress;
    settings
        .save(&Some(tempdir.path().join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;

    let mut child = standalone_daemon(&settings.shared).await?;
    let shared = &settings.shared;

    // Kill the daemon and wait for it to shut down.
    assert_success(shutdown_daemon(shared).await?);
    wait_for_shutdown(&mut child).await?;

    // Boot it up again
    let mut child = standalone_daemon(&settings.shared).await?;

    assert_group_status(
        shared,
        PUEUE_DEFAULT_GROUP,
        GroupStatus::Running,
        "Default group should still be running.",
    )
    .await?;

    child.kill()?;
    Ok(())
}

/// The daemon should start in the same state as before shutdown, if no tasks are queued.
/// This function tests for the paused state.
#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn test_start_paused(#[case] compress: bool) -> Result<()> {
    let (mut settings, tempdir) = daemon_base_setup()?;
    settings.daemon.compress_state_file = compress;
    settings
        .save(&Some(tempdir.path().join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;

    let mut child = standalone_daemon(&settings.shared).await?;
    let shared = &settings.shared;

    // This pauses the daemon
    pause_tasks(shared, TaskSelection::All).await?;

    // Kill the daemon and wait for it to shut down.
    assert_success(shutdown_daemon(shared).await?);
    wait_for_shutdown(&mut child).await?;

    // Boot it up again
    let mut child = standalone_daemon(&settings.shared).await?;

    assert_group_status(
        shared,
        PUEUE_DEFAULT_GROUP,
        GroupStatus::Paused,
        "Default group should still be paused.",
    )
    .await?;

    child.kill()?;
    Ok(())
}

/// Groups with queued tasks are paused on restore, unless they're listed in
/// `keep_running_on_restore`.
#[rstest]
#[case(false, GroupStatus::Paused)]
#[case(true, GroupStatus::Running)]
#[tokio::test]
async fn test_restore_with_queued_tasks(
    #[case] keep_running: bool,
    #[case] expected: GroupStatus,
) -> Result<()> {
    let (mut settings, tempdir) = daemon_base_setup()?;
    if keep_running {
        settings.daemon.keep_running_on_restore = vec![PUEUE_DEFAULT_GROUP.into()];
    }
    settings
        .save(&Some(tempdir.path().join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;

    let mut child = standalone_daemon(&settings.shared).await?;
    let shared = &settings.shared;

    // The default group runs one task at a time, so the second one stays queued.
    assert_success(add_task(shared, "sleep 60").await?);
    assert_success(add_task(shared, "sleep 60").await?);
    wait_for_task_condition(shared, 0, Task::is_running).await?;

    assert_success(shutdown_daemon(shared).await?);
    wait_for_shutdown(&mut child).await?;

    let mut child = standalone_daemon(&settings.shared).await?;
    assert_group_status(
        shared,
        PUEUE_DEFAULT_GROUP,
        expected,
        "Group status after restore didn't match.",
    )
    .await?;

    child.kill()?;
    Ok(())
}
