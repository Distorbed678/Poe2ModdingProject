//! Loads the SDK's real `hello-server` example library (a cdylib built by the SDK, as a mod
//! author would) and runs its `!hello` command through the host. Skipped, with a note, when the
//! SDK has not been built (`cd third_party/Poe2ModSDK && cargo build --release`).

use std::path::PathBuf;

use poe_cluster::ModDirectory;
use poe_config::ModsConfig;
use poe_mod_host::{
    EVENT_COMMAND, EventData, HostConfig, InstanceSnap, ModCommand, ModHost, PlayerSnap, store,
};

fn sdk() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/Poe2ModSDK")
}

#[test]
fn the_sdk_hello_server_mod_loads_and_answers_hello() {
    let sdk = sdk();
    let lib = sdk.join("target/release/libhello_server.so");
    let manifest = sdk.join("examples/hello-server/mod.toml");
    if !lib.is_file() || !manifest.is_file() || !cfg!(target_os = "linux") {
        eprintln!("skipped: build the SDK first ({})", lib.display());
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mods");
    let m = dir.join("hello-server");
    std::fs::create_dir_all(m.join("server")).unwrap();
    std::fs::copy(&manifest, m.join("mod.toml")).unwrap();
    std::fs::copy(&lib, m.join("server/libhello_server.so")).unwrap();
    store::write_state(&dir, &["hello-server".into()]).unwrap();

    let host = ModHost::load(&HostConfig {
        mods: ModsConfig {
            dir: dir.clone(),
            ..Default::default()
        },
        game_version: "0.5.5e".into(),
        server_version: "0.1.0".into(),
        server_dir: tmp.path().to_owned(),
        reserved_commands: vec!["help".into(), "where".into()],
    });
    assert_eq!(
        host.native_mods(),
        [("hello-server".to_owned(), "0.1.0".to_owned())]
    );
    let names: Vec<_> = host.commands().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["hello"]);
    // Server-only: nothing for clients.
    assert!(host.adverts().is_empty());
    assert_eq!(host.server_only(), 1);

    let ann = PlayerSnap {
        id: 41,
        entity: 12001,
        account: "ann".into(),
        character: "Ann".into(),
        admin: false,
        x: 3.0,
        y: 4.0,
    };
    let snap = InstanceSnap {
        frame: 9,
        instance_id: 2,
        area: "1_1_1".into(),
        players: vec![ann.clone()],
    };
    let ev = EventData {
        kind: EVENT_COMMAND,
        frame: 9,
        instance_id: 2,
        snapshot: Some(&snap),
        player: Some(&ann),
        text: "hello",
        args: "",
    };
    let out = host.dispatch_command("hello-server", &ev);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(
        out[0].command,
        ModCommand::SendChat {
            target: 41,
            text: "Hello, Ann!".into()
        }
    );
}

#[test]
fn a_mod_for_another_server_api_or_game_version_is_refused_but_the_server_goes_on() {
    let sdk = sdk();
    let lib = sdk.join("target/release/libhello_server.so");
    let manifest = sdk.join("examples/hello-server/mod.toml");
    if !lib.is_file() || !manifest.is_file() || !cfg!(target_os = "linux") {
        eprintln!("skipped: build the SDK first");
        return;
    }
    let text = std::fs::read_to_string(&manifest).unwrap();
    for (what, edited, game) in [
        (
            "another server API",
            text.replace("server_api = 1", "server_api = 2"),
            "0.5.5e",
        ),
        (
            "an older server API",
            text.replace("server_api = 1", "server_api = 0"),
            "0.5.5e",
        ),
        ("another game version", text.clone(), "0.6.0"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mods");
        let m = dir.join("hello-server");
        std::fs::create_dir_all(m.join("server")).unwrap();
        std::fs::write(m.join("mod.toml"), edited).unwrap();
        std::fs::copy(&lib, m.join("server/libhello_server.so")).unwrap();
        store::write_state(&dir, &["hello-server".into()]).unwrap();
        let host = ModHost::load(&HostConfig {
            mods: ModsConfig {
                dir,
                ..Default::default()
            },
            game_version: game.into(),
            server_version: "0.1.0".into(),
            server_dir: tmp.path().to_owned(),
            reserved_commands: vec![],
        });
        let s = host.status();
        assert_eq!(s[0].state, "skipped", "{what}");
        assert!(
            host.native_mods().is_empty(),
            "{what}: the library must not be mapped"
        );
        assert!(host.commands().is_empty(), "{what}");
        eprintln!("{what}: {}", s[0].detail);
    }
}
