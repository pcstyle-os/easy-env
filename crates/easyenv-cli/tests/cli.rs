use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

fn easyenv() -> Command {
    Command::cargo_bin("easyenv").expect("binary exists")
}

fn apply_test_env(command: &mut Command, app_home: &std::path::Path) {
    command
        .env("EASYENV_HOME", app_home)
        .env("EASYENV_SECRET_BACKEND", "file");
}

#[test]
fn init_set_get_list_import_and_doctor_work() {
    let temp = tempdir().unwrap();
    let app_home = temp.path().join("app");
    let project = temp.path().join("project");
    let nested = project.join("nested");
    fs::create_dir_all(&nested).unwrap();

    let mut init = easyenv();
    apply_test_env(&mut init, &app_home);
    init.args(["init", project.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized project"));

    let mut set_global = easyenv();
    apply_test_env(&mut set_global, &app_home);
    set_global
        .current_dir(temp.path())
        .args(["set", "GLOBAL_TOKEN=global-123"])
        .assert()
        .success();

    let mut set_project = easyenv();
    apply_test_env(&mut set_project, &app_home);
    set_project
        .current_dir(&project)
        .args(["set", "PROJECT_TOKEN=project-456"])
        .assert()
        .success();

    let mut get_project = easyenv();
    apply_test_env(&mut get_project, &app_home);
    get_project
        .current_dir(&nested)
        .args(["get", "PROJECT_TOKEN", "--reveal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PROJECT_TOKEN=project-456"));

    let mut list = easyenv();
    apply_test_env(&mut list, &app_home);
    let output = list
        .current_dir(&nested)
        .args(["list", "--format", "json", "--reveal"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: Value = serde_json::from_slice(&output).unwrap();
    let entries = payload.as_array().unwrap();
    assert_eq!(entries.len(), 2);

    fs::write(project.join(".env"), "FROM_DOTENV=dotenv-value\n").unwrap();
    let mut import = easyenv();
    apply_test_env(&mut import, &app_home);
    import
        .current_dir(&project)
        .args(["import", ".env", "--delete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1 variables"));
    assert!(!project.join(".env").exists());

    let mut get_imported = easyenv();
    apply_test_env(&mut get_imported, &app_home);
    get_imported
        .current_dir(&project)
        .args(["get", "FROM_DOTENV", "--reveal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("FROM_DOTENV=dotenv-value"));

    let mut doctor = easyenv();
    apply_test_env(&mut doctor, &app_home);
    let doctor_output = doctor
        .current_dir(&project)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&doctor_output).unwrap();
    assert_eq!(report["checks"].as_array().unwrap().len(), 5);
}

#[test]
fn exec_injects_resolved_environment() {
    let temp = tempdir().unwrap();
    let app_home = temp.path().join("app");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();

    let mut init = easyenv();
    apply_test_env(&mut init, &app_home);
    init.args(["init", project.to_str().unwrap()])
        .assert()
        .success();

    let mut set = easyenv();
    apply_test_env(&mut set, &app_home);
    set.current_dir(&project)
        .args(["set", "OPENAI_API_KEY=test-secret"])
        .assert()
        .success();

    let mut exec = easyenv();
    apply_test_env(&mut exec, &app_home);
    exec.current_dir(&project)
        .args(["exec", "--", "env"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OPENAI_API_KEY=test-secret"));
}

#[test]
fn workspace_push_and_pull_roundtrip_between_two_homes() {
    let temp = tempdir().unwrap();
    let sender_home = temp.path().join("sender-home");
    let receiver_home = temp.path().join("receiver-home");
    let sender_project = temp.path().join("sender-project");
    let receiver_project = temp.path().join("receiver-project");
    let bundle_path = temp.path().join("share.age");
    fs::create_dir_all(&sender_project).unwrap();
    fs::create_dir_all(&receiver_project).unwrap();

    let mut sender_init = easyenv();
    apply_test_env(&mut sender_init, &sender_home);
    sender_init
        .args(["init", sender_project.to_str().unwrap()])
        .assert()
        .success();

    let mut receiver_init = easyenv();
    apply_test_env(&mut receiver_init, &receiver_home);
    receiver_init
        .args(["init", receiver_project.to_str().unwrap()])
        .assert()
        .success();

    let mut set = easyenv();
    apply_test_env(&mut set, &sender_home);
    set.current_dir(&sender_project)
        .args(["set", "OPENAI_API_KEY=shared-secret"])
        .assert()
        .success();

    let mut workspace = easyenv();
    apply_test_env(&mut workspace, &receiver_home);
    let workspace_output = workspace
        .args(["workspace", "show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let workspace_payload: Value = serde_json::from_slice(&workspace_output).unwrap();
    let recipient = workspace_payload["recipient"].as_str().unwrap().to_string();
    assert!(recipient.starts_with("age1"));

    let mut push = easyenv();
    apply_test_env(&mut push, &sender_home);
    push.current_dir(&sender_project)
        .args([
            "push",
            "--to",
            &recipient,
            "--key",
            "OPENAI_API_KEY",
            "--out",
            bundle_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrote encrypted share bundle"));
    assert!(bundle_path.exists());

    let mut pull = easyenv();
    apply_test_env(&mut pull, &receiver_home);
    pull.current_dir(&receiver_project)
        .args(["pull", bundle_path.to_str().unwrap(), "--project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1 shared secret"));

    let mut get = easyenv();
    apply_test_env(&mut get, &receiver_home);
    get.current_dir(&receiver_project)
        .args(["get", "OPENAI_API_KEY", "--reveal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OPENAI_API_KEY=shared-secret"));
}

#[test]
fn walkthrough_teaches_the_main_flow_and_supports_walktrough_alias() {
    let temp = tempdir().unwrap();
    let app_home = temp.path().join("app");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();

    let mut init = easyenv();
    apply_test_env(&mut init, &app_home);
    init.args(["init", project.to_str().unwrap()])
        .assert()
        .success();

    let mut walkthrough = easyenv();
    apply_test_env(&mut walkthrough, &app_home);
    walkthrough
        .current_dir(&project)
        .args(["walktrough"])
        .assert()
        .success()
        .stdout(predicate::str::contains("easyenv walkthrough"))
        .stdout(predicate::str::contains("Step 1: initialize a project"))
        .stdout(predicate::str::contains("easyenv exec -- your-command"))
        .stdout(predicate::str::contains("easyenv push --to age1..."))
        .stdout(predicate::str::contains("Project status:   initialized"));
}

#[test]
fn completion_generates_scripts() {
    let temp = tempdir().unwrap();
    let app_home = temp.path().join("app");

    let mut completion = easyenv();
    apply_test_env(&mut completion, &app_home);
    completion
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("easyenv"));
}
