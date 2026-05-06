use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

struct TestEnv {
    home: TempDir,
    project: TempDir,
}

impl TestEnv {
    fn new() -> Self {
        Self {
            home: TempDir::new().unwrap(),
            project: TempDir::new().unwrap(),
        }
    }

    fn cmd(&self) -> Command {
        let mut command = Command::cargo_bin("easyenv").unwrap();
        command
            .env("EASYENV_STORE", "file")
            .env("EASYENV_HOME", self.home.path())
            .env_remove("FOO")
            .env_remove("GLOBAL_KEY")
            .env_remove("PROJECT_KEY")
            .current_dir(self.project.path());
        command
    }
}

#[test]
fn stores_and_reveals_global_secret() {
    let env = TestEnv::new();

    env.cmd()
        .args(["set", "GLOBAL_KEY=secret", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stored GLOBAL_KEY in global"));

    env.cmd()
        .args(["get", "GLOBAL_KEY", "--reveal"])
        .assert()
        .success()
        .stdout("secret\n");
}

#[test]
fn masks_values_by_default() {
    let env = TestEnv::new();

    env.cmd()
        .args(["set", "GLOBAL_KEY=abcdef", "--global"])
        .assert()
        .success();

    env.cmd()
        .args(["get", "GLOBAL_KEY"])
        .assert()
        .success()
        .stdout("**cdef\n");
}

#[test]
fn resolves_shell_before_project_before_global() {
    let env = TestEnv::new();

    env.cmd().arg("init").assert().success();
    env.cmd()
        .args(["set", "FOO=global", "--global"])
        .assert()
        .success();
    env.cmd().args(["set", "FOO=project"]).assert().success();

    env.cmd()
        .args(["get", "FOO", "--reveal"])
        .env("FOO", "shell")
        .assert()
        .success()
        .stdout("shell\n");

    env.cmd()
        .args(["get", "FOO", "--reveal"])
        .assert()
        .success()
        .stdout("project\n");
}

#[test]
fn injects_environment_into_child_process() {
    let env = TestEnv::new();

    env.cmd()
        .args(["set", "GLOBAL_KEY=injected", "--global"])
        .assert()
        .success();

    let script = if cfg!(windows) {
        vec!["exec", "--", "cmd", "/C", "echo %GLOBAL_KEY%"]
    } else {
        vec!["exec", "--", "sh", "-c", "printf %s \"$GLOBAL_KEY\""]
    };

    env.cmd()
        .args(script)
        .assert()
        .success()
        .stdout(predicate::str::contains("injected"));
}

#[test]
fn imports_dotenv_values() {
    let env = TestEnv::new();
    let dotenv_path = env.project.path().join(".env");
    std::fs::write(&dotenv_path, "FOO=bar\nPROJECT_KEY=\"quoted\"\n").unwrap();

    env.cmd()
        .args(["import", ".env", "--global"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 2 values into global"));

    env.cmd()
        .args(["list", "--scope", "global", "--format", "dotenv"])
        .assert()
        .success()
        .stdout(predicate::str::contains("FOO=bar"))
        .stdout(predicate::str::contains("PROJECT_KEY=quoted"));
}

#[test]
fn explains_resolution() {
    let env = TestEnv::new();

    env.cmd()
        .args(["set", "GLOBAL_KEY=secret", "--global"])
        .assert()
        .success();

    env.cmd()
        .args(["get", "GLOBAL_KEY", "--explain"])
        .assert()
        .success()
        .stdout(predicate::str::contains("GLOBAL_KEY resolved from global"))
        .stdout(predicate::str::contains("global found"));
}
