use std::fs::read_to_string;

use pueue_lib::Task;

use crate::{helper::*, internal_prelude::*};

/// Make sure that callback commands are executed while variables are
/// templated into the command as expected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_callback_variables() -> Result<()> {
    let (mut settings, tempdir) = daemon_base_setup()?;

    // Configure the daemon to use a callback command that echos some variables into a file
    // that's located in the temporary runtime directory of the daemon.
    let tempdir_path = tempdir.path().to_path_buf();
    let echo_command =
        "echo '{{queued_count}}\n{{stashed_count}}\n{{command}}\n{{id}}\n{{result}}'";
    settings.daemon.callback = Some(format!(
        "{echo_command} > {}/testfile",
        tempdir_path.to_string_lossy()
    ));
    settings
        .save(&Some(tempdir_path.join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;

    // Create the daemon with the changed settings.
    let daemon = daemon_with_settings(settings, tempdir).await?;
    let shared = &daemon.settings.shared;

    // Create one stashed task.
    assert_success(create_stashed_task(shared, "stashed", None).await?);
    // Create a task that'll then trigger the callback
    assert_success(add_task(shared, "ls").await?);

    // Give the callback command some time to be executed.
    sleep_ms(3000).await;

    let callback_output = read_to_string(tempdir_path.join("testfile"))?;

    assert_eq!(callback_output, "0\n1\nls\n1\nSuccess\n");

    Ok(())
}

/// Tasks that were still running when the daemon died get their callback on the next start.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_callback_for_interrupted_task() -> Result<()> {
    let (mut settings, tempdir) = daemon_base_setup()?;
    let tempdir_path = tempdir.path().to_path_buf();
    settings.daemon.callback = Some(format!(
        "echo '{{{{id}}}} {{{{result}}}}' >> {}/testfile",
        tempdir_path.to_string_lossy()
    ));
    settings
        .save(&Some(tempdir_path.join("pueue.yml")))
        .context("Couldn't write pueue config to temporary directory")?;

    let mut child = standalone_daemon(&settings.shared).await?;
    let shared = &settings.shared;

    assert_success(add_task(shared, "sleep 60").await?);
    wait_for_task_condition(shared, 0, Task::is_running).await?;

    // Kill the daemon without giving it a chance to handle its task.
    child.kill()?;
    child.wait()?;

    let mut child = standalone_daemon(&settings.shared).await?;
    sleep_ms(1000).await;

    let callback_output = read_to_string(tempdir_path.join("testfile"))?;
    assert_eq!(callback_output, "0 Killed\n");

    child.kill()?;
    Ok(())
}
