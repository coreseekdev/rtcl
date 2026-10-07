//! 命令批量注册。宿主一行挂载：`rtcl_data::register(&mut interp);`
//!
//! 注册名即宿主契约：`yaml` / `yaml::decode` / `yaml::encode` / `sha1` /
//! `sha256` / `atomic-write` / `url::normalize` / `url::key` / `url::pubkey`。

use rtcl_core::command::CommandFunc;
use rtcl_core::interp::Interp;

pub fn register(interp: &mut Interp) {
    let cmds: &[(&str, CommandFunc)] = &[
        ("yaml", crate::yaml::cmd_yaml),
        ("yaml::decode", crate::yaml::cmd_yaml_decode),
        ("yaml::encode", crate::yaml::cmd_yaml_encode),
        ("sha1", crate::sha::cmd_sha1),
        ("sha256", crate::sha::cmd_sha256),
        ("atomic-write", crate::fs::cmd_atomic_write),
        ("url::normalize", crate::url::cmd_url_normalize),
        ("url::key", crate::url::cmd_url_key),
        ("url::pubkey", crate::url::cmd_url_pubkey),
    ];
    for (name, func) in cmds {
        interp.register_command(name, *func);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册冒烟：9 个命令名全部经 registry 查询 API 在场。
    #[test]
    fn registers_all_nine_commands() {
        let mut interp = Interp::new();
        register(&mut interp);
        for name in [
            "yaml",
            "yaml::decode",
            "yaml::encode",
            "sha1",
            "sha256",
            "atomic-write",
            "url::normalize",
            "url::key",
            "url::pubkey",
        ] {
            assert!(interp.command_exists(name), "{name} not registered");
        }
        assert!(!interp.command_exists("url"), "url ensemble 未挂载，契约外");
        assert!(!interp.command_exists("sha3"), "无关命令不受影响");
    }
}
