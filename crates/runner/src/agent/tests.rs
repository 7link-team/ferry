use super::*;

#[tokio::test]
async fn managed_workspaces_isolate_conversations_and_agents_and_preserve_resume_state() {
    let instance = tempfile::tempdir().unwrap();
    let other_instance = tempfile::tempdir().unwrap();
    let first = workspace(instance.path(), "1-2").await.unwrap();
    tokio::fs::write(first.join("keep.txt"), "resume")
        .await
        .unwrap();
    for path in [
        workspace(instance.path(), "3-2").await.unwrap(),
        workspace(instance.path(), "1-4").await.unwrap(),
        workspace(other_instance.path(), "1-2").await.unwrap(),
    ] {
        assert_ne!(path, first);
        assert!(!path.join("keep.txt").exists());
    }
    assert_eq!(workspace(instance.path(), "1-2").await.unwrap(), first);
    assert_eq!(
        tokio::fs::read_to_string(first.join("keep.txt"))
            .await
            .unwrap(),
        "resume"
    );
    for key in ["", "../1-2", "1-2/..", "1-2-3", "0-2", "C:\\1-2", "1-a"] {
        assert!(workspace(instance.path(), key).await.is_err(), "{key}");
    }
}
use ferry_protocol::agent::*;
use serde_json::json;

#[test]
fn directory_aliases_share_one_kernel_lock_until_the_execution_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let held = directory_lock(dir.path()).unwrap();
    assert!(directory_lock(&dir.path().join(".")).is_err());
    drop(held);
    assert!(directory_lock(dir.path()).is_ok());
}

#[cfg(unix)]
#[test]
fn concurrent_child_before_exec_cannot_extend_a_released_directory_lock() {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::{net::UnixStream, process::CommandExt};

    let dir = tempfile::tempdir().unwrap();
    let held = directory_lock(dir.path()).unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    let parent_fd = parent.as_raw_fd();
    parent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let process = std::thread::spawn(move || {
        let fd = child.as_raw_fd();
        let result = unsafe {
            std::process::Command::new("sh")
                .args(["-c", "exit 0"])
                .pre_exec(move || {
                    // Only async-signal-safe syscalls between fork and exec.
                    libc::close(parent_fd);
                    let mut byte = 1u8;
                    if libc::write(fd, (&byte as *const u8).cast(), 1) != 1
                        || libc::read(fd, (&mut byte as *mut u8).cast(), 1) != 1
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                })
                .status()
        };
        drop(child);
        result
    });
    let mut ready = [0];
    parent.read_exact(&mut ready).unwrap();
    drop(held);
    let reacquired = directory_lock(dir.path()).is_ok();
    parent.write_all(&[1]).unwrap();
    assert!(process.join().unwrap().unwrap().success());
    assert!(
        reacquired,
        "a pre-exec child must not retain the released lock"
    );
}

fn request(runtime: Runtime) -> Request {
    Request {
        binding: None,
        runtime,
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        prompt: "hello".into(),
        model: None,
        native_session_id: None,
        permission: Permission::OnRequest,
        timeout_seconds: 10,
        platform_tools: false,
        provider: None,
    }
}

fn acp_ready(runtime: Runtime, permission: Permission) -> protocol::Protocol {
    acp_ready_platform(runtime, permission, false)
}
fn acp_ready_platform(
    runtime: Runtime,
    permission: Permission,
    platform_tools: bool,
) -> protocol::Protocol {
    let mut r = request(runtime);
    r.permission = permission;
    r.platform_tools = platform_tools;
    let mut p = protocol::Protocol::new(r, "acp-test".into());
    assert_eq!(p.initial()["params"]["protocolVersion"], 1);
    assert_eq!(
        p.receive(
            json!({"id":0,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}})
        )
        .unwrap()
        .writes[0]["method"],
        "session/new"
    );
    let step = p.receive(json!({"id":1,"result":{"sessionId":"native-acp","modes":{"availableModes":[{"id":"default"},{"id":"build"},{"id":"plan"}]}}})).unwrap();
    assert_eq!(
        step.writes[0]["params"]["modeId"],
        if permission == Permission::ReadOnly {
            "plan"
        } else if runtime == Runtime::Opencode {
            // OpenCode asks first in build mode; the Runner forces permission "ask".
            "build"
        } else {
            "default"
        }
    );
    assert_eq!(
        p.receive(json!({"id":2,"result":{}})).unwrap().writes[0]["method"],
        "session/prompt"
    );
    p
}

#[test]
fn acp_platform_transport_is_per_call_and_does_not_unlock_local_tools() {
    for runtime in [Runtime::Gemini, Runtime::Qwen, Runtime::Opencode] {
        let mut p = acp_ready_platform(runtime, Permission::ReadOnly, true);
        let event = |title: &str| json!({"id":90,"method":"session/request_permission","params":{"sessionId":"native-acp","toolCall":{"title":title},"options":[{"kind":"allow_always","optionId":"forever"},{"kind":"allow_once","optionId":"once"}]}});
        let step = p.receive(event("ferry_request")).unwrap();
        assert_eq!(step.writes[0]["result"]["outcome"]["optionId"], "once");
        assert!(step.events.is_empty());
        assert_eq!(
            p.receive(event("Bash")).unwrap().writes[0]["result"]["outcome"]["outcome"],
            "cancelled"
        );
    }
}

#[test]
fn acp_engines_stream_resume_and_never_convert_once_into_permanent_approval() {
    for runtime in [Runtime::Gemini, Runtime::Qwen, Runtime::Opencode] {
        let mut p = acp_ready(runtime, Permission::OnRequest);
        let step = p.receive(json!({"method":"session/update","params":{"sessionId":"native-acp","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}})).unwrap();
        assert!(matches!(&step.events[0], Event::Text {text,replace:false,..} if text == "hello"));
        let step = p.receive(json!({"id":10,"method":"session/request_permission","params":{"sessionId":"native-acp","toolCall":{"title":"Write"},"options":[{"kind":"allow_always","optionId":"all"},{"kind":"allow_once","optionId":"once"}]}})).unwrap();
        let Event::Approval { request_id, .. } = &step.events[0] else {
            panic!("approval missing")
        };
        let request_id = request_id.clone();
        assert_eq!(
            p.input(Input::Decision {
                request_id: request_id.clone(),
                allow: true
            })
            .unwrap()
            .writes[0]["result"]["outcome"]["optionId"],
            "once"
        );
        assert!(
            p.input(Input::Decision {
                request_id,
                allow: true
            })
            .is_err()
        );
        assert_eq!(
            p.receive(json!({"id":4,"result":{"stopReason":"end_turn"}}))
                .unwrap()
                .done,
            Some(Outcome::Completed)
        );
        let mut r = request(runtime);
        r.native_session_id = Some("native-acp".into());
        let mut p = protocol::Protocol::new(r, "resume".into());
        assert_eq!(p.receive(json!({"id":0,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}})).unwrap().writes[0]["method"],"session/load");
        assert!(
            p.receive(json!({"id":1,"error":{"code":-1,"message":"private native diagnostic"}}))
                .is_err()
        );
    }
}

#[test]
fn acp_readonly_cancels_tools_and_rejects_foreign_sessions_or_missing_safe_modes() {
    let mut p = acp_ready(Runtime::Gemini, Permission::ReadOnly);
    assert_eq!(p.receive(json!({"id":9,"method":"session/request_permission","params":{"sessionId":"native-acp"}})).unwrap().writes[0]["result"]["outcome"]["outcome"],"cancelled");
    assert!(
        p.receive(json!({"method":"session/update","params":{"sessionId":"foreign","update":{}}}))
            .is_err()
    );
    let mut p = protocol::Protocol::new(request(Runtime::Qwen), "modes".into());
    p.receive(json!({"id":0,"result":{"protocolVersion":1}}))
        .unwrap();
    assert!(
        p.receive(
            json!({"id":1,"result":{"sessionId":"a","modes":{"availableModes":[{"id":"yolo"}]}}})
        )
        .is_err()
    );
}

fn dsh_initialize() -> Value {
    json!({"id":0,"result":{"protocolVersion":1,
        "agentInfo":{"name":"deepseek-harness-acp"},
        "agentCapabilities":{"sessionCapabilities":{"resume":{}}}}})
}

fn dsh_options(values: &[(&str, &str)]) -> Value {
    json!([{"id":"model","type":"select","currentValue":"[\"provider\",\"model\"]",
        "options":[{"group":"provider","options":values.iter().map(|(provider, model)|
            json!({"value":serde_json::to_string(&[provider, model]).unwrap(),"name":model}))
            .collect::<Vec<_>>()}]}])
}

fn dsh_policy(p: &mut protocol::Protocol) {
    p.receive(json!({"method":"agent_runtime/permissions","params":{"sessionId":"dsh-session","preset":"read-only"}})).unwrap();
}

fn dsh_ready(permission: Permission, platform_tools: bool) -> protocol::Protocol {
    let mut r = request(Runtime::Dsh);
    r.permission = permission;
    r.platform_tools = platform_tools;
    let mut p = protocol::Protocol::new(r, "dsh-test".into());
    assert_eq!(
        p.receive(dsh_initialize()).unwrap().writes[0]["method"],
        "session/new"
    );
    dsh_policy(&mut p);
    let next = p
        .receive(json!({"id":1,"result":{"sessionId":"dsh-session","configOptions":[]}}))
        .unwrap();
    assert_eq!(next.writes[0]["method"], "session/prompt");
    assert!(
        matches!(&next.events[0], Event::Session { native_session_id } if native_session_id == "dsh-session")
    );
    p
}

#[test]
fn dsh_new_resume_stream_and_completion_use_its_advertised_contract() {
    assert_eq!(Runtime::parse("dsh"), Some(Runtime::Dsh));
    let cmd = command(&request(Runtime::Dsh));
    assert_eq!(cmd.as_std().get_args().last().unwrap(), "acp");
    let mut p = dsh_ready(Permission::ReadOnly, false);
    let step = p.receive(json!({"method":"session/update","params":{"sessionId":"dsh-session",
        "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}}})).unwrap();
    assert!(matches!(&step.events[0], Event::Text {text,..} if text == "answer"));
    p.receive(json!({"method":"agent_runtime/turn_end","params":{
        "sessionId":"dsh-session","reason":"completed"}}))
        .unwrap();
    assert_eq!(
        p.receive(json!({"id":4,"result":{"stopReason":"end_turn"}}))
            .unwrap()
            .done,
        Some(Outcome::Completed)
    );
    let mut r = request(Runtime::Dsh);
    r.native_session_id = Some("dsh-session".into());
    let mut p = protocol::Protocol::new(r, "resume".into());
    assert_eq!(
        p.receive(dsh_initialize()).unwrap().writes[0]["method"],
        "session/resume"
    );
    dsh_policy(&mut p);
    assert_eq!(
        p.receive(json!({"id":1,"result":{"configOptions":[]}}))
            .unwrap()
            .writes[0]["method"],
        "session/prompt"
    );
    assert!(
        p.receive(json!({"method":"session/update","params":{"sessionId":"another","update":{}}}))
            .is_err()
    );
    let mut p = protocol::Protocol::new(request(Runtime::Dsh), "wrong-agent".into());
    assert!(
        p.receive(json!({"id":0,"result":{"protocolVersion":1,"agentInfo":{"name":"other"}}}))
            .is_err()
    );
}

#[test]
fn dsh_model_selection_rejects_missing_ambiguous_and_unacknowledged_routes() {
    for (requested, values, accepted) in [
        (
            "provider/model",
            vec![("provider", "model"), ("other", "model")],
            true,
        ),
        (
            "model",
            vec![("provider", "model"), ("other", "model")],
            false,
        ),
        ("missing", vec![("provider", "model")], false),
        ("model", vec![("provider", "model")], true),
    ] {
        let mut r = request(Runtime::Dsh);
        r.model = Some(requested.into());
        let mut p = protocol::Protocol::new(r, "model".into());
        p.receive(dsh_initialize()).unwrap();
        dsh_policy(&mut p);
        let step = p.receive(json!({"id":1,"result":{"sessionId":"dsh-session","configOptions":dsh_options(&values)}}));
        if accepted {
            let step = step.unwrap();
            assert_eq!(step.writes[0]["method"], "session/set_config_option");
            assert_eq!(
                step.writes[0]["params"]["value"],
                "[\"provider\",\"model\"]"
            );
            assert!(
                p.receive(json!({"id":3,"result":{"configOptions":[]}}))
                    .is_err()
            );
        } else {
            assert!(step.is_err());
        }
    }
    let mut r = request(Runtime::Dsh);
    r.model = Some("provider/model".into());
    let mut p = protocol::Protocol::new(r, "model-success".into());
    p.receive(dsh_initialize()).unwrap();
    dsh_policy(&mut p);
    let options = dsh_options(&[("provider", "model")]);
    p.receive(json!({"id":1,"result":{"sessionId":"dsh-session","configOptions":options}}))
        .unwrap();
    assert_eq!(
        p.receive(json!({"id":3,"result":{"configOptions":options}}))
            .unwrap()
            .writes[0]["method"],
        "session/prompt"
    );
}

#[test]
fn dsh_id_only_permissions_cannot_unlock_unrelated_tools_or_other_sessions() {
    let permission = |id: &str| {
        json!({"id":90,"method":"session/request_permission","params":{
        "sessionId":"dsh-session","toolCall":{"toolCallId":id},
        "options":[{"kind":"allow_once","optionId":"once"},{"kind":"reject_once","optionId":"reject"}]}})
    };
    let mut p = dsh_ready(Permission::ReadOnly, true);
    p.receive(json!({"method":"session/update","params":{"sessionId":"dsh-session","update":{
        "sessionUpdate":"tool_call","toolCallId":"platform","title":"mcp__ferry__ferry_request","status":"pending"}}})).unwrap();
    assert_eq!(
        p.receive(permission("platform")).unwrap().writes[0]["result"]["outcome"]["optionId"],
        "once"
    );
    assert_eq!(
        p.receive(permission("unknown")).unwrap().writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    p.receive(
        json!({"method":"session/update","params":{"sessionId":"dsh-session","update":{
        "sessionUpdate":"tool_call_update","toolCallId":"platform","status":"completed"}}}),
    )
    .unwrap();
    assert_eq!(
        p.receive(permission("platform")).unwrap().writes[0]["result"]["outcome"]["outcome"],
        "cancelled"
    );
    let mut p = dsh_ready(Permission::OnRequest, false);
    let step = p.receive(permission("local")).unwrap();
    let Event::Approval { request_id, .. } = &step.events[0] else {
        panic!("missing approval")
    };
    let request_id = request_id.clone();
    assert_eq!(
        p.input(Input::Decision {
            request_id: request_id.clone(),
            allow: true
        })
        .unwrap()
        .writes[0]["result"]["outcome"]["optionId"],
        "once"
    );
    assert!(
        p.input(Input::Decision {
            request_id,
            allow: true
        })
        .is_err()
    );
}

#[test]
fn dsh_native_end_reason_overrides_acp_success_and_requires_a_matching_receipt() {
    for reason in ["error", "blocked", "aborted", "max-tokens", "unknown"] {
        let mut p = dsh_ready(Permission::ReadOnly, false);
        p.receive(json!({"method":"agent_runtime/turn_end","params":{
            "sessionId":"dsh-session","reason":reason}}))
            .unwrap();
        assert_eq!(
            p.receive(json!({"id":4,"result":{"stopReason":"end_turn"}}))
                .unwrap()
                .done,
            Some(Outcome::Failed)
        );
    }
    let mut p = dsh_ready(Permission::ReadOnly, false);
    assert!(
        p.receive(json!({"id":4,"result":{"stopReason":"end_turn"}}))
            .is_err()
    );
    p.receive(json!({"method":"agent_runtime/turn_end","params":{
        "sessionId":"foreign","reason":"completed"}}))
        .unwrap();
    assert!(
        p.receive(json!({"id":4,"result":{"stopReason":"end_turn"}}))
            .is_err()
    );
}

#[test]
fn dsh_fails_closed_without_policy_receipt_or_resume_capability() {
    let mut p = protocol::Protocol::new(request(Runtime::Dsh), "missing-policy".into());
    p.receive(dsh_initialize()).unwrap();
    assert!(
        p.receive(json!({"id":1,"result":{"sessionId":"dsh-session","configOptions":[]}}))
            .is_err()
    );
    let mut p = protocol::Protocol::new(request(Runtime::Dsh), "foreign-policy".into());
    p.receive(dsh_initialize()).unwrap();
    p.receive(json!({"method":"agent_runtime/permissions","params":{"sessionId":"foreign","preset":"read-only"}})).unwrap();
    assert!(
        p.receive(json!({"id":1,"result":{"sessionId":"dsh-session"}}))
            .is_err()
    );
    let mut r = request(Runtime::Dsh);
    r.native_session_id = Some("dsh-session".into());
    let mut p = protocol::Protocol::new(r, "missing-resume".into());
    assert!(p.receive(json!({"id":0,"result":{"protocolVersion":1,"agentInfo":{"name":"deepseek-harness-acp"}}})).is_err());
}

#[test]
fn pi_agent_end_preserves_final_text_and_failed_outcomes() {
    for failed in [false, true] {
        let mut r = request(Runtime::Pi);
        r.permission = Permission::ReadOnly;
        let mut p = protocol::Protocol::new(r, "pi".into());
        assert_eq!(p.initial()["type"], "get_state");
        assert_eq!(p.receive(json!({"id":"initialize","type":"response","success":true,"data":{"sessionId":"native-pi"}})).unwrap().writes[0]["type"],"prompt");
        p.receive(json!({"type":"message_start","message":{"role":"assistant"}}))
            .unwrap();
        let s=p.receive(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"answer"}],"stopReason":if failed{"error"}else{"stop"},"usage":{"input":2,"output":3}}})).unwrap();
        assert!(matches!(&s.events[0],Event::Text{text,replace:true,..} if text=="answer"));
        assert_eq!(
            p.receive(json!({"type":"agent_end","messages":[]}))
                .unwrap()
                .done,
            Some(if failed {
                Outcome::Failed
            } else {
                Outcome::Completed
            })
        );
    }
    let mut r = request(Runtime::Pi);
    r.permission = Permission::ReadOnly;
    let args = command(&r)
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for flag in [
        "--no-tools",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
    ] {
        assert!(args.contains(&flag.to_owned()));
    }
    assert!(!args.contains(&"--no-mcp".to_owned()));
    r.native_session_id = Some("saved-session".into());
    let args = command(&r)
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--session", "saved-session"])
    );
    r.permission = Permission::OnRequest;
    assert!(validate(&r).is_err());
}

fn codex() -> protocol::Protocol {
    let mut p = protocol::Protocol::new(request(Runtime::Codex), "invocation-a".into());
    p.receive(json!({"id":0,"result":{}})).unwrap();
    p.receive(json!({"id":1,"result":{"thread":{"id":"thread-a"}}}))
        .unwrap();
    p.receive(
        json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}}),
    )
    .unwrap();
    p
}

#[test]
fn codex_approval_is_single_item_and_cannot_be_reused() {
    for allow in [false, true] {
        let mut p = codex();
        let step = p
            .receive(
                json!({"id":"native-approval","method":"item/commandExecution/requestApproval",
            "params":{"threadId":"thread-a","turnId":"turn-a","command":"write marker"}}),
            )
            .unwrap();
        let Event::Approval { request_id, .. } = &step.events[0] else {
            panic!()
        };
        let input = Input::Decision {
            request_id: request_id.clone(),
            allow,
        };
        let step = p.input(input.clone()).unwrap();
        assert_eq!(
            step.writes[0],
            json!({"id":"native-approval","result":{"decision":if allow {"accept"} else {"decline"}}})
        );
        assert!(p.input(input).is_err());
        assert!(
            p.input(Input::Decision {
                request_id: "old-invocation:1".into(),
                allow: true
            })
            .is_err()
        );
    }
}

#[test]
fn codex_fails_closed_for_foreign_events_and_unknown_requests() {
    let mut p = codex();
    assert!(p.receive(json!({"method":"item/agentMessage/delta","params":{"threadId":"private-thread","turnId":"turn-a","itemId":"a","delta":"secret"}})).is_err());
    assert!(p.receive(json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"old-turn","status":"completed"}}})).is_err());
    assert!(p.receive(json!({"id":88,"method":"item/permissions/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a"}})).is_err());
    assert!(p.receive(json!({"id":89,"method":"item/tool/requestUserInput","params":{"threadId":"thread-a","turnId":"turn-a"}})).is_err());
}

#[test]
fn native_resume_is_explicit_and_different_identity_is_rejected() {
    for runtime in [Runtime::Codex, Runtime::Claude] {
        let mut req = request(runtime);
        req.native_session_id = Some("expected".into());
        let mut p = protocol::Protocol::new(req, "a".into());
        if runtime == Runtime::Codex {
            let step = p.receive(json!({"id":0,"result":{}})).unwrap();
            assert_eq!(step.writes[1]["method"], "thread/resume");
            assert_eq!(step.writes[1]["params"]["threadId"], "expected");
            assert!(
                p.receive(json!({"id":1,"result":{"thread":{"id":"wrong"}}}))
                    .is_err()
            );
        } else {
            assert!(
                p.receive(json!({"type":"system","subtype":"init","session_id":"wrong"}))
                    .is_err()
            );
        }
    }
}

#[test]
fn codex_delta_final_replacement_and_usage_are_not_duplicated() {
    let mut p = codex();
    let delta=p.receive(json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-a","turnId":"turn-a","itemId":"message-a","delta":"hello"}})).unwrap();
    assert!(matches!(&delta.events[0],Event::Text{text,replace:false,..} if text=="hello"));
    let final_value = json!({"method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"message-a","type":"agentMessage","text":"hello"}}});
    assert!(p.receive(final_value).unwrap().events.is_empty());
    p.receive(json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-a","tokenUsage":{"last":{"inputTokens":12,"outputTokens":4},"total":{"inputTokens":999}}}})).unwrap();
    assert_eq!(p.usage().unwrap().input_tokens, 12);
    let done=p.receive(json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"completed"}}})).unwrap();
    assert_eq!(done.done, Some(Outcome::Completed));
}

#[test]
fn resumed_thread_usage_does_not_replace_current_turn_usage() {
    let mut req = request(Runtime::Codex);
    req.native_session_id = Some("thread-a".into());
    let mut p = protocol::Protocol::new(req, "a".into());
    p.receive(json!({"id":0,"result":{}})).unwrap();
    p.receive(json!({"id":1,"result":{"thread":{"id":"thread-a"}}}))
        .unwrap();
    let step = p.receive(json!({"method":"thread/tokenUsage/updated","params":{
        "threadId":"thread-a","turnId":"previous-turn","tokenUsage":{"last":{"inputTokens":999,"outputTokens":999}}}})).unwrap();
    assert!(step.events.is_empty());
    assert!(p.usage().is_none());
}

#[test]
fn claude_tool_decisions_preserve_input_and_never_grant_a_session() {
    for allow in [true, false] {
        let mut p = protocol::Protocol::new(request(Runtime::Claude), "a".into());
        p.receive(json!({"type":"control_response","response":{"request_id":"initialize","subtype":"success"}})).unwrap();
        p.receive(json!({"type":"system","subtype":"init","session_id":"claude-a"}))
            .unwrap();
        let params = json!({"file_path":"marker.txt","content":"approved"});
        let step=p.receive(json!({"type":"control_request","request_id":"tool-a","request":{"subtype":"can_use_tool","tool_name":"Write","input":params}})).unwrap();
        let Event::Approval { request_id, .. } = &step.events[0] else {
            panic!()
        };
        let response = p
            .input(Input::Decision {
                request_id: request_id.clone(),
                allow,
            })
            .unwrap();
        assert_eq!(response.writes[0]["response"]["request_id"], "tool-a");
        assert_eq!(
            response.writes[0]["response"]["response"]["behavior"],
            if allow { "allow" } else { "deny" }
        );
        if allow {
            assert_eq!(
                response.writes[0]["response"]["response"]["updatedInput"],
                params
            );
        }
    }
}

#[test]
fn read_only_does_not_accept_runtime_tool_approvals() {
    let mut p = codex();
    // Use an otherwise valid initialized session with a read-only policy.
    let mut req = request(Runtime::Codex);
    req.permission = Permission::ReadOnly;
    let mut readonly = protocol::Protocol::new(req, "b".into());
    let step = readonly.receive(json!({"id":0,"result":{}})).unwrap();
    assert_eq!(step.writes[1]["params"]["sandbox"], "read-only");
    assert_eq!(step.writes[1]["params"]["approvalPolicy"], "never");
    readonly
        .receive(json!({"id":1,"result":{"thread":{"id":"thread-a"}}}))
        .unwrap();
    readonly.receive(json!({"method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a"}}})).unwrap();
    assert!(readonly.receive(json!({"id":3,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a"}})).is_err());
    // P0 never exposes arbitrary permissions or full-access execution.
    assert!(p.receive(json!({"id":9,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a","availableDecisions":["acceptForSession"]}})).is_err());
}

#[tokio::test]
async fn bounded_json_reader_rejects_oversize_and_truncated_output() {
    for bytes in [
        vec![b'x'; LINE_LIMIT + 2],
        b"{\"incomplete\":true}".to_vec(),
    ] {
        let mut reader = BufReader::new(bytes.as_slice());
        let mut buffer = Vec::new();
        assert!(read(&mut reader, &mut buffer).await.is_err());
        assert!(buffer.len() <= LINE_LIMIT + 1);
    }
}

#[tokio::test]
async fn partial_json_is_preserved_across_input_selection() {
    let (mut writer, reader) = tokio::io::duplex(100);
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();
    writer.write_all(b"{\"id\":").await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), read(&mut reader, &mut buffer))
            .await
            .is_err()
    );
    writer.write_all(b"1}\n").await.unwrap();
    assert_eq!(
        read(&mut reader, &mut buffer).await.unwrap(),
        Some(json!({"id":1}))
    );
}

fn fixture(mode: &str) -> Command {
    let mut c = Command::new("node");
    c.arg(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent-runtime.mjs"),
    )
    .arg(mode);
    c
}

#[tokio::test]
async fn process_requires_completion_and_cancellation_works_under_output_backpressure() {
    for mode in ["eof", "malformed", "hang", "flood"] {
        let req = request(Runtime::Codex);
        let (events, mut received) = mpsc::channel(1);
        let (_input, inputs) = mpsc::channel(2);
        let (stop, cancel) = watch::channel(false);
        let task = tokio::spawn(run_command(req, fixture(mode), events, inputs, cancel));
        if mode == "hang" || mode == "flood" {
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(10), received.recv())
                    .await
                    .unwrap(),
                Some(Event::Session { .. })
            ));
            stop.send(true).unwrap();
        } else {
            // Drain while running to avoid hiding a protocol error behind a full queue.
            while received.recv().await.is_some() {}
        }
        let result = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        if mode == "hang" || mode == "flood" {
            assert_eq!(result.unwrap(), Outcome::Cancelled);
        } else {
            assert!(result.is_err(), "{mode} must not count as successful");
        }
    }
}

#[tokio::test]
async fn timeout_and_consumer_disconnect_stop_the_runtime() {
    for disconnect in [false, true] {
        let mut req = request(Runtime::Codex);
        req.timeout_seconds = 2;
        let (events, mut received) = mpsc::channel(8);
        let (_input, inputs) = mpsc::channel(2);
        let (_stop, cancel) = watch::channel(false);
        let task = tokio::spawn(run_command(req, fixture("hang"), events, inputs, cancel));
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(10), received.recv())
                .await
                .unwrap(),
            Some(Event::Session { .. })
        ));
        if disconnect {
            drop(received);
        }
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome,
            if disconnect {
                Outcome::Cancelled
            } else {
                Outcome::TimedOut
            }
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn cancellation_reaps_native_runtime_descendants() {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    let (events, mut received) = mpsc::channel(8);
    let (_input, inputs) = mpsc::channel(2);
    let (stop, cancel) = watch::channel(false);
    let task = tokio::spawn(run_command(
        request(Runtime::Codex),
        fixture("descendant"),
        events,
        inputs,
        cancel,
    ));
    let pid = loop {
        if let Some(Event::Text { text, .. }) =
            tokio::time::timeout(Duration::from_secs(10), received.recv())
                .await
                .unwrap()
        {
            break text.parse::<u32>().unwrap();
        }
    };
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!handle.is_null());
    assert_eq!(
        unsafe { WaitForSingleObject(handle, 0) },
        258,
        "child must be running before cancellation"
    );
    stop.send(true).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ended = unsafe { WaitForSingleObject(handle, 5000) };
    unsafe {
        CloseHandle(handle);
    }
    assert_eq!(outcome, Outcome::Cancelled);
    assert_eq!(
        ended, 0,
        "cancellation cannot finish with a descendant alive"
    );
}

#[tokio::test]
async fn fixture_process_checks_full_handshake_tool_decision_and_end_event() {
    let (events, mut received) = mpsc::channel(8);
    let (input, inputs) = mpsc::channel(2);
    let (_stop, cancel) = watch::channel(false);
    let task = tokio::spawn(run_command(
        request(Runtime::Codex),
        fixture("complete"),
        events,
        inputs,
        cancel,
    ));
    let mut text = String::new();
    let mut approvals = 0;
    let mut finished = false;
    while let Some(event) = received.recv().await {
        match event {
            Event::Approval { request_id, .. } => {
                approvals += 1;
                input
                    .send(Input::Decision {
                        request_id,
                        allow: true,
                    })
                    .await
                    .unwrap();
            }
            Event::Text { text: value, .. } => text.push_str(&value),
            Event::Finished { outcome, .. } => {
                assert_eq!(outcome, Outcome::Completed);
                finished = true;
            }
            _ => {}
        }
    }
    assert_eq!(task.await.unwrap().unwrap(), Outcome::Completed);
    assert_eq!(approvals, 1);
    assert_eq!(text, "fixture reply");
    assert!(finished);
}

// Explicitly opt-in: uses native user login and may incur model usage. It never
// changes the native CLI configuration, and all prompts use an isolated directory.
#[tokio::test]
#[ignore = "requires installed, authenticated native CLI; real model usage"]
async fn live_native_roundtrip_and_resume() {
    let runtime = match std::env::var("AGENT_TEST_RUNTIME").as_deref() {
        Ok("claude") => Runtime::Claude,
        _ => Runtime::Codex,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(runtime);
    req.cwd = dir.path().to_string_lossy().into_owned();
    req.permission = Permission::ReadOnly;
    req.timeout_seconds = 120;
    req.prompt="Remember this exact marker: violet-pine-7329. Reply with the marker only. Do not use tools.".into();
    let mut native = None;
    for turn in 0..2 {
        if turn == 1 {
            req.prompt="What exact marker did I ask you to remember? Reply with the marker only. Do not use tools.".into();
            req.native_session_id = native.clone();
        }
        let (events, mut received) = mpsc::channel(16);
        let (_input, inputs) = mpsc::channel(2);
        let (_stop, cancel) = watch::channel(false);
        let task = tokio::spawn(run(req.clone(), events, inputs, cancel));
        let mut text = String::new();
        while let Some(event) = received.recv().await {
            match event {
                Event::Session { native_session_id } => native = Some(native_session_id),
                Event::Text {
                    text: value,
                    replace,
                    ..
                } => {
                    if replace {
                        text = value;
                    } else {
                        text.push_str(&value);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(task.await.unwrap().unwrap(), Outcome::Completed);
        assert!(
            text.contains("violet-pine-7329"),
            "actual model answer: {text}"
        );
        assert!(native.is_some());
    }
}

#[tokio::test]
#[ignore = "requires installed, authenticated native CLI; writes only an isolated test file"]
async fn live_native_tool_allow_and_deny() {
    let runtime = match std::env::var("AGENT_TEST_RUNTIME").as_deref() {
        Ok("claude") => Runtime::Claude,
        _ => Runtime::Codex,
    };
    for allow in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("approval-marker.txt");
        let mut req = request(runtime);
        req.cwd = dir.path().to_string_lossy().into_owned();
        req.timeout_seconds = 120;
        req.prompt = if runtime == Runtime::Claude {
            "Use the Write tool to create approval-marker.txt containing exactly APPROVED. Do not use Bash or any other writing tool. If permission is denied, stop and report it; do not retry or work around the denial.".into()
        } else {
            "Use the shell tool to run exactly this command in the current directory: powershell.exe -NoProfile -NonInteractive -Command \"[System.IO.File]::WriteAllText('approval-marker.txt','APPROVED')\". Do not use apply_patch or any other writing tool. If permission is denied, stop and report it; do not retry or work around the denial.".into()
        };
        let (events, mut received) = mpsc::channel(16);
        let (input, inputs) = mpsc::channel(2);
        let (_stop, cancel) = watch::channel(false);
        let task = tokio::spawn(run(req, events, inputs, cancel));
        let mut approvals = 0;
        while let Some(event) = received.recv().await {
            if let Event::Approval { request_id, .. } = event {
                approvals += 1;
                input
                    .send(Input::Decision { request_id, allow })
                    .await
                    .unwrap();
            }
        }
        let outcome = task.await.unwrap().unwrap();
        assert_eq!(outcome, Outcome::Completed);
        assert!(
            approvals > 0,
            "native tool must reach the Human decision bridge"
        );
        if allow {
            assert_eq!(std::fs::read_to_string(marker).unwrap(), "APPROVED");
        } else {
            assert!(!marker.exists(), "a denied tool must not write the file");
        }
    }
}

#[tokio::test]
#[ignore = "requires installed, authenticated native CLI; real model usage"]
async fn live_native_cancel_pending_tool() {
    let runtime = match std::env::var("AGENT_TEST_RUNTIME").as_deref() {
        Ok("claude") => Runtime::Claude,
        _ => Runtime::Codex,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(runtime);
    req.cwd = dir.path().to_string_lossy().into_owned();
    req.timeout_seconds = 120;
    req.prompt = if runtime == Runtime::Claude {
        "Use the Write tool to create cancelled-marker.txt containing CANCELLED. Do not use Bash or other tools.".into()
    } else {
        "Use the shell tool to run exactly: powershell.exe -NoProfile -NonInteractive -Command \"[System.IO.File]::WriteAllText('cancelled-marker.txt','CANCELLED')\". Do not use apply_patch or other writing tools.".into()
    };
    let (events, mut received) = mpsc::channel(16);
    let (_input, inputs) = mpsc::channel(2);
    let (stop, cancel) = watch::channel(false);
    let task = tokio::spawn(run(req, events, inputs, cancel));
    let mut asked = false;
    while let Some(event) = received.recv().await {
        if matches!(event, Event::Approval { .. }) {
            asked = true;
            stop.send(true).unwrap();
            break;
        }
    }
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(asked, "must cancel an actual native permission request");
    assert_eq!(outcome, Outcome::Cancelled);
    assert!(!dir.path().join("cancelled-marker.txt").exists());
}

// Opt-in acceptance against a local mock upstream (data/maintenance/cli-e2e):
// FERRY_E2E_UPSTREAM=http://127.0.0.1:18431 FERRY_E2E_RUNTIMES=claude,codex,...
// Proves each real CLI reaches the configured address, key and model through
// its generated isolated configuration, and never reads the user's own login.
#[tokio::test]
#[ignore = "requires installed native CLIs and a local mock upstream"]
async fn live_custom_provider_roundtrip() {
    let upstream = std::env::var("FERRY_E2E_UPSTREAM").expect("FERRY_E2E_UPSTREAM");
    let runtimes = std::env::var("FERRY_E2E_RUNTIMES").expect("FERRY_E2E_RUNTIMES");
    let mut failures = Vec::new();
    for name in runtimes.split(',').filter(|v| !v.is_empty()) {
        let runtime = Runtime::parse(name).expect("known runtime");
        let format = runtime.provider_formats()[0];
        let base = match format {
            "gemini" => upstream.clone(),
            _ => format!("{upstream}/v1"),
        };
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(runtime);
        req.cwd = dir.path().to_string_lossy().into_owned();
        req.permission = if runtime == Runtime::Pi {
            Permission::ReadOnly
        } else {
            Permission::OnRequest
        };
        req.timeout_seconds = 180;
        req.model = Some(format!("e2e-{name}"));
        req.prompt = "Reply with the word ready. Do not use tools.".into();
        req.provider = Some(ferry_protocol::agent::Provider {
            key: "4242".into(),
            format: format.into(),
            base_url: base,
            api_key: format!("e2e-key-{name}"),
            models: vec![format!("e2e-{name}")],
            options: serde_json::json!({"format": format}),
        });
        let (events, mut received) = mpsc::channel(64);
        let (_input, inputs) = mpsc::channel(2);
        let (_stop, cancel) = watch::channel(false);
        let task = tokio::spawn(run(req, events, inputs, cancel));
        let mut text = String::new();
        while let Some(event) = received.recv().await {
            if let Event::Text {
                text: value,
                replace,
                ..
            } = event
            {
                if replace {
                    text = value;
                } else {
                    text.push_str(&value);
                }
            }
        }
        let outcome = task.await.unwrap();
        let ok = matches!(outcome, Ok(Outcome::Completed)) && text.contains("FERRY-E2E-OK");
        println!("E2E {name}: outcome={outcome:?} text={text:?}");
        if !ok {
            failures.push(name.to_owned());
        }
    }
    assert!(failures.is_empty(), "failed: {failures:?}");
}

#[test]
fn acp_selects_config_option_modes_and_ignores_vendor_extensions() {
    // OpenCode 1.19 exposes modes as a configOptions select, not `modes`.
    let mut p = protocol::Protocol::new(request(Runtime::Opencode), "acp-test".into());
    p.receive(json!({"id":0,"result":{"protocolVersion":1}}))
        .unwrap();
    let step = p
        .receive(json!({"id":1,"result":{"sessionId":"s","configOptions":[{"id":"mode","options":[{"value":"build"},{"value":"plan"}]}]}}))
        .unwrap();
    assert_eq!(step.writes[0]["method"], "session/set_config_option");
    assert_eq!(step.writes[0]["params"]["value"], "build");
    // Grok sends `_x.ai/*` extensions before admission; requests get method-not-found.
    let mut g = protocol::Protocol::new(request(Runtime::Grok), "acp-test".into());
    g.receive(json!({"id":0,"result":{"protocolVersion":1}}))
        .unwrap();
    assert!(
        g.receive(json!({"jsonrpc":"2.0","method":"_x.ai/session/setup","params":{}}))
            .unwrap()
            .writes
            .is_empty()
    );
    let reply = g
        .receive(json!({"jsonrpc":"2.0","id":9,"method":"_x.ai/ask","params":{}}))
        .unwrap();
    assert_eq!(reply.writes[0]["error"]["code"], -32601);
    // Grok has no modes: on-request goes straight to the prompt; read-only is refused.
    let step = g
        .receive(json!({"id":1,"result":{"sessionId":"g"}}))
        .unwrap();
    assert_eq!(step.writes[0]["method"], "session/prompt");
    let mut read_only = request(Runtime::Grok);
    read_only.permission = Permission::ReadOnly;
    let mut g = protocol::Protocol::new(read_only, "acp-test".into());
    g.receive(json!({"id":0,"result":{"protocolVersion":1}}))
        .unwrap();
    assert!(
        g.receive(json!({"id":1,"result":{"sessionId":"g"}}))
            .is_err()
    );
}
