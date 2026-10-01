use pretty_assertions::assert_eq;
use pueue_lib::{GroupStatus, message::*, task::*};
use rstest::rstest;

use crate::{helper::*, internal_prelude::*};

/// Test if killing running tasks works as intended.
///
/// We test different ways of killing those tasks.
/// - Via the --all flag, which just kills everything.
/// - Via the --group flag, which just kills everything in the default group.
/// - Via specific ids.
///
/// If a whole group or everything is killed, the respective groups should also be paused,
/// as long as there's no further queued task.
/// This is security measure to prevent unwanted task execution in an emergency.
#[rstest]
#[case(
    KillRequest {
        tasks: TaskSelection::All,
        signal: None,
    }, true
)]
#[case(
    KillRequest {
        tasks: TaskSelection::Group(PUEUE_DEFAULT_GROUP.into()),
        signal: None,
    }, true
)]
#[case(
    KillRequest {
        tasks: TaskSelection::TaskIds(vec![0, 1, 2]),
        signal: None,
    }, false
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_kill_tasks_with_pause(
    #[case] kill_message: KillRequest,
    #[case] group_should_pause: bool,
) -> Result<()> {
    let daemon = daemon().await?;
    let shared = &daemon.settings.shared;

    // Add multiple tasks and start them immediately
    for _ in 0..3 {
        assert_success(add_and_start_task(shared, "sleep 60").await?);
    }
    // Wait until all tasks are running, they should be start `immediately`.
    for id in 0..3 {
        assert_task_condition(
            shared,
            id,
            Task::is_running,
            "Tasks should start immediately.",
        )
        .await?;
    }

    // Add another task that will be normally enqueued.
    for _ in 0..3 {
        assert_success(add_task(shared, "sleep 60").await?);
    }

    // Send the kill message
    send_request(shared, kill_message).await?;

    // Make sure all tasks get killed
    for id in 0..3 {
        wait_for_task_condition(shared, id, |task| {
            matches!(
                task.status,
                TaskStatus::Done {
                    result: TaskResult::Killed,
                    ..
                }
            )
        })
        .await?;
    }

    // Groups should be paused in specific modes.
    if group_should_pause {
        let state = get_state(shared).await?;
        assert_eq!(
            state.groups.get(PUEUE_DEFAULT_GROUP).unwrap().status,
            GroupStatus::Paused
        );
    }

    Ok(())
}

/// This test ensures the following rule:
/// If a whole group or everything is killed, the respective groups should not be paused, as long
/// as there's no further queued task in that group.
///
/// We test different ways of killing those tasks.
/// - Via the --all flag, which just kills everything.
/// - Via the --group flag, which just kills everything in the default group.
/// - Via specific ids.
#[rstest]
#[case(
    KillRequest {
        tasks: TaskSelection::All,
        signal: None,
    }
)]
#[case(
    KillRequest {
        tasks: TaskSelection::Group(PUEUE_DEFAULT_GROUP.into()),
        signal: None,
    }
)]
#[case(
    KillRequest {
        tasks: TaskSelection::TaskIds(vec![0, 1, 2]),
        signal: None,
    }
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_kill_tasks_without_pause(#[case] kill_message: KillRequest) -> Result<()> {
    let daemon = daemon().await?;
    let shared = &daemon.settings.shared;

    // Add multiple tasks and start them immediately
    for _ in 0..3 {
        assert_success(add_and_start_task(shared, "sleep 60").await?);
    }
    // Wait until all tasks are running, they should be start `immediately`.
    for id in 0..3 {
        assert_task_condition(
            shared,
            id,
            Task::is_running,
            "Tasks should start immediately",
        )
        .await?;
    }

    // Add a dummy group that also shouldn't be paused.
    add_group_with_slots(shared, "testgroup", 1).await?;

    // Send the kill message
    send_request(shared, kill_message).await?;

    // Make sure all tasks get killed
    for id in 0..3 {
        wait_for_task_condition(shared, id, |task| {
            matches!(
                task.status,
                TaskStatus::Done {
                    result: TaskResult::Killed,
                    ..
                }
            )
        })
        .await?;
    }

    // Groups should not be paused, since no other queued tasks exist at this point in time.
    let state = get_state(shared).await?;
    assert_eq!(
        state.groups.get(PUEUE_DEFAULT_GROUP).unwrap().status,
        GroupStatus::Running
    );
    assert_eq!(
        state.groups.get("testgroup").unwrap().status,
        GroupStatus::Running
    );

    Ok(())
}

fn is_killed(task: &Task) -> bool {
    matches!(
        task.status,
        TaskStatus::Done {
            result: TaskResult::Killed,
            ..
        }
    )
}

async fn daemon_with_graceful_kill(timeout: u64) -> Result<PueueDaemon> {
    let (mut settings, tempdir) = daemon_base_setup()?;
    settings.daemon.graceful_kill_timeout = Some(timeout);
    settings
        .save(&Some(tempdir.path().join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;
    daemon_with_settings(settings, tempdir).await
}

/// Wait until the task has set up its signal handling and printed `ready`.
async fn wait_until_ready(shared: &pueue_lib::settings::Shared, task_id: usize) -> Result<()> {
    for _ in 0..100 {
        if get_task_log(shared, task_id, None).await?.contains("ready") {
            return Ok(());
        }
        sleep_ms(50).await;
    }
    bail!("Task {task_id} didn't get ready")
}

fn kill_request(task_id: usize) -> KillRequest {
    KillRequest {
        tasks: TaskSelection::TaskIds(vec![task_id]),
        signal: None,
    }
}

/// With a graceful kill timeout, the task gets a SIGTERM it can handle.
/// It counts as killed, even though it exits with a non-zero code by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_graceful_kill() -> Result<()> {
    let daemon = daemon_with_graceful_kill(60).await?;
    let shared = &daemon.settings.shared;

    assert_success(
        add_and_start_task(
            shared,
            "trap 'echo got sigterm; exit 3' TERM; echo ready; sleep 60 & wait",
        )
        .await?,
    );
    wait_until_ready(shared, 0).await?;

    send_request(shared, kill_request(0)).await?;
    wait_for_task_condition(shared, 0, is_killed).await?;

    let log = get_task_log(shared, 0, None).await?;
    assert!(
        log.contains("got sigterm"),
        "Task should handle SIGTERM. Got: {log}"
    );

    Ok(())
}

/// A task that ignores the SIGTERM is killed once the timeout runs out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_graceful_kill_timeout() -> Result<()> {
    let daemon = daemon_with_graceful_kill(2).await?;
    let shared = &daemon.settings.shared;

    assert_success(add_and_start_task(shared, "trap '' TERM; echo ready; sleep 60").await?);
    wait_until_ready(shared, 0).await?;

    send_request(shared, kill_request(0)).await?;
    sleep_ms(1000).await;
    assert!(
        get_task(shared, 0).await?.is_running(),
        "Task should ignore the SIGTERM until the timeout runs out."
    );

    wait_for_task_condition(shared, 0, is_killed).await?;

    Ok(())
}

/// Killing a task again while it waits for its timeout kills it right away.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_graceful_kill_twice() -> Result<()> {
    let daemon = daemon_with_graceful_kill(60).await?;
    let shared = &daemon.settings.shared;

    assert_success(add_and_start_task(shared, "trap '' TERM; echo ready; sleep 60").await?);
    wait_until_ready(shared, 0).await?;

    send_request(shared, kill_request(0)).await?;
    sleep_ms(500).await;
    assert!(get_task(shared, 0).await?.is_running());

    send_request(shared, kill_request(0)).await?;
    wait_for_task_condition(shared, 0, is_killed).await?;

    Ok(())
}
