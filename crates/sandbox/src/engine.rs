//! QuickJS is linked only into this executable, never the business server.
use ferry_protocol::sandbox::{Batch, ErrorCode, MAX_RESPONSE_BYTES, Reply, VERSION};
use rquickjs::{Context, Ctx, Function, Module, Runtime};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BOOTSTRAP: &str = r#"
globalThis.__deepFreeze = function(value, seen = new Set()) {
    if (value === null || (typeof value !== 'object' && typeof value !== 'function') || seen.has(value)) return value;
    seen.add(value);
    for (const descriptor of Object.values(Object.getOwnPropertyDescriptors(value))) {
        if ('value' in descriptor) __deepFreeze(descriptor.value, seen);
    }
    return Object.freeze(value);
};
// Input is parsed JSON, so there are no cycles, accessors or functions.
globalThis.__freezeData = function(value) {
    if (value !== null && typeof value === 'object') {
        for (const key of Object.keys(value)) __freezeData(value[key]);
        Object.freeze(value);
    }
    return value;
};
globalThis.__serialize = function(value) {
    return JSON.stringify(value, function(key, item) {
        const type = typeof item;
        if (type === 'number' && !Number.isFinite(item)) throw new Error('non-JSON number');
        if (type === 'undefined' || type === 'bigint' || type === 'function' || type === 'symbol') throw new Error('non-JSON value');
        if (item instanceof Promise) throw new Error('async expression');
        return item;
    });
};
// Native dates stay inside this closure. Only frozen scalar values cross it.
globalThis.__dateTime = (() => {
    const NativeDate = Date;
    class FrozenDateTime {
        constructor(value) {
            const match = typeof value?.iso === 'string' && /^([+-]\d{6}|\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{3})(Z|([+-])(\d{2}):(\d{2}))$/.exec(value.iso);
            if (!match || typeof value.zone !== 'string') throw new Error('invalid frozen date');
            const offset = match[8] === 'Z' ? 0 : (match[9] === '-' ? -1 : 1) * (Number(match[10])*60 + Number(match[11]));
            const millis = NativeDate.parse(value.iso);
            if (!Number.isFinite(millis) || Math.abs(offset) >= 1440 ||
                new NativeDate(millis + offset*60000).toISOString() !== value.iso.slice(0, value.iso.length - match[8].length) + 'Z') throw new Error('invalid frozen date');
            Object.assign(this, {iso:value.iso, zoneName:value.zone, offset,
                year:Number(match[1]),month:Number(match[2]),day:Number(match[3]),
                hour:Number(match[4]),minute:Number(match[5]),second:Number(match[6]),millisecond:Number(match[7]),millis});
            Object.freeze(this);
        }
        toISO() { return this.iso; }
        toISODate() { return this.iso.slice(0,this.iso.indexOf('T')); }
        toMillis() { return this.millis; }
        toString() { return this.iso; }
        toJSON() { return this.iso; }
        valueOf() { return this.millis; }
    }
    __deepFreeze(FrozenDateTime);
    const create = value => value === undefined ? undefined : new FrozenDateTime(value);
    create.is = value => value instanceof FrozenDateTime;
    return create;
})();
globalThis.__prepareContext = function(value) {
    if (value.$now !== undefined) value.$now = __dateTime(value.$now);
    if (value.$today !== undefined) value.$today = __dateTime(value.$today);
    return __freezeData(value);
};
globalThis.__compile = function(source, template, syntaxOnly, capturePresence) {
    const prelude = "'use strict'; const {$node,$execution,$workflow,$vars,$now,$today} = __context; const {$json,$binary,$input,$} = __bindings(__scope,__index,__context); ";
    const compile = expression => {
        __check('export default (\n' + expression + ');\n');
        const result = capturePresence
            ? 'const value = (\n' + expression + '\n); return __serialize(value === undefined ? {present:false} : {present:true,value});'
            : 'return __serialize((\n' + expression + '));\n';
        return syntaxOnly ? function(){} : new Function('__context','__scope','__index', prelude + result);
    };
    if (!template) return compile(source);
    if (!source.startsWith('=')) throw new SyntaxError('expected expression');
    const text = source.slice(1);
    if (!text.includes('{{')) return compile(text);
    const parts = [];
    let at = 0, sole = null;
    while (at < text.length) {
        const start = text.indexOf('{{', at);
        if (start < 0) { parts.push(JSON.stringify(text.slice(at))); break; }
        if (start > at) parts.push(JSON.stringify(text.slice(at, start)));
        let end = text.indexOf('}}', start + 2), expression;
        for (; end >= 0; end = text.indexOf('}}', end + 1)) {
            expression = text.slice(start + 2, end);
            try { compile(expression); break; }
            catch (error) { if (!(error instanceof SyntaxError)) throw error; }
        }
        if (end < 0) throw new SyntaxError('unclosed expression');
        if (start === 0 && end + 2 === text.length) sole = expression;
        parts.push('__text((' + expression + '))');
        at = end + 2;
    }
    return compile(sole === null ? parts.join('+') : sole);
};
globalThis.__text = function(value) {
    // Validate before converting, so interpolation cannot disguise a Promise,
    // undefined, non-finite number or non-JSON nested value as ordinary text.
    const encoded = __serialize(value);
    return __dateTime.is(value) ? value.toISO() : typeof value === 'string' ? value : encoded;
};
globalThis.__itemError = function(ambiguous = false) {
    const error = new Error();
    Object.defineProperty(error, 'name', {value:ambiguous ? 'FerryAmbiguousItem' : 'FerryMissingItem'});
    throw error;
};
globalThis.__scope = function(data) {
    __freezeData(data);
    const identities = new Map(), names = new Map();
    for (const output of data.upstream) {
        identities.set(JSON.stringify([output.node,output.iteration_hash]), output);
        if (!names.has(output.name)) names.set(output.name, []);
        names.get(output.name).push(output);
    }
    const scope = {items:data.items, lookup(name, current) {
        const outputs = names.get(name);
        if (!outputs) return __itemError();
        const node = outputs[0].node;
        if (outputs.some(output => output.node !== node)) return __itemError(true);
        // Each lookup gets fresh helper functions. Tenant properties on a helper
        // cannot carry state into another evaluation in this batch.
        const all = () => Object.freeze(outputs.flatMap(output => output.items));
        return Object.freeze({all, first:() => all()[0], get item() {
            if (!current) return __itemError();
            const pending = [...(current.paired || [])], visited = new Set(), found = new Map();
            while (pending.length) {
                const pair = pending.pop();
                const key = JSON.stringify([pair.node,pair.iteration_hash,pair.index]);
                if (visited.has(key)) continue;
                visited.add(key);
                const output = identities.get(JSON.stringify([pair.node,pair.iteration_hash]));
                const item = output && output.items[pair.index];
                if (!item) return __itemError();
                if (pair.node === node) found.set(key, item);
                else for (const parent of item.paired || []) pending.push(parent);
            }
            if (found.size !== 1) return __itemError(found.size > 1);
            return found.values().next().value;
        }});
    }};
    Object.freeze(scope.lookup);
    return Object.freeze(scope);
};
globalThis.__bindings = function(scope, index, context) {
    if (!scope) return {$json:context.$json,$binary:context.$binary,$input:context.$input,$:()=>__itemError()};
    const item = index === null ? undefined : scope.items[index];
    return {$json:item && item.json,$binary:item && (item.binary || Object.freeze({})),
        $input:Object.freeze({all:()=>scope.items,first:()=>scope.items[0],get item() {
            return item || __itemError();
        }}), $:name=>scope.lookup(name,item)};
};
"#;
const PURE: &str = r#"
Math.random = function() { throw new Error('random is unavailable in deterministic evaluation'); };
globalThis.Date = undefined;
globalThis.performance = undefined;
globalThis.crypto = undefined;
"#;

fn classify(
    ctx: &Ctx<'_>,
    error: rquickjs::Error,
    timed_out: &AtomicBool,
    fallback: ErrorCode,
) -> ErrorCode {
    if timed_out.load(Ordering::Relaxed) {
        return ErrorCode::Timeout;
    }
    if matches!(error, rquickjs::Error::Allocation) {
        return ErrorCode::Memory;
    }
    if error.is_exception() {
        let exception = ctx.catch();
        if let Some(object) = exception.as_object() {
            match object.get::<_, String>("name").ok().as_deref() {
                Some("FerryAmbiguousItem") => return ErrorCode::AmbiguousItem,
                Some("FerryMissingItem") => return ErrorCode::MissingItem,
                _ => {}
            }
        }
        if exception
            .as_object()
            .and_then(|object| object.get::<_, String>("message").ok())
            .as_deref()
            == Some("out of memory")
        {
            return ErrorCode::Memory;
        }
    }
    fallback
}

pub(crate) fn evaluate(batch: Batch) -> Reply {
    let deadline = Instant::now() + Duration::from_millis(batch.timeout_ms);
    let timed_out = Arc::new(AtomicBool::new(false));
    let runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(_) => return Reply::error(ErrorCode::Memory, None),
    };
    runtime.set_memory_limit(batch.memory_bytes);
    runtime.set_max_stack_size(256 * 1024);
    let interrupted = timed_out.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let expired = Instant::now() >= deadline;
        if expired {
            interrupted.store(true, Ordering::Relaxed);
        }
        expired
    })));
    let context = match Context::full(&runtime) {
        Ok(context) => context,
        Err(_) => return Reply::error(ErrorCode::Memory, None),
    };
    context.with(|ctx| {
        // Module declaration uses JS_EVAL_FLAG_COMPILE_ONLY. This callback has
        // no I/O or state, and does not evaluate any module or user expression.
        let check = Function::new(ctx.clone(), |ctx: Ctx<'_>, source: String| {
            Module::declare(ctx, "<expression-syntax>", source).map(|_| ())
        });
        if check
            .and_then(|check| ctx.globals().set("__check", check))
            .is_err()
        {
            return Reply::error(ErrorCode::Internal, None);
        }
        if let Err(error) = ctx.eval::<(), _>(BOOTSTRAP) {
            return Reply::error(classify(&ctx, error, &timed_out, ErrorCode::Internal), None);
        }
        if batch.deterministic {
            if let Err(error) = ctx.eval::<(), _>(PURE) {
                return Reply::error(classify(&ctx, error, &timed_out, ErrorCode::Internal), None);
            }
        }
        // User expressions cannot alter globals/intrinsics for a later item.
        if let Err(error) = ctx.eval::<(), _>(
            "__deepFreeze(globalThis); __deepFreeze(Object.getPrototypeOf(globalThis));",
        ) {
            return Reply::error(classify(&ctx, error, &timed_out, ErrorCode::Internal), None);
        }
        let compile: Function = match ctx.globals().get("__compile") {
            Ok(compile) => compile,
            Err(_) => return Reply::error(ErrorCode::Internal, None),
        };
        let mut functions = HashMap::<(&str, bool, bool), Function>::new();
        let freeze: Function = match ctx.globals().get("__prepareContext") {
            Ok(freeze) => freeze,
            Err(_) => return Reply::error(ErrorCode::Internal, None),
        };
        let input = |value: &serde_json::Value| {
            serde_json::to_vec(value)
                .ok()
                .and_then(|bytes| ctx.json_parse(bytes).ok())
                .ok_or(rquickjs::Error::Allocation)
                .and_then(|value| freeze.call::<_, rquickjs::Value>((value,)))
        };
        let shared = if batch.syntax_only {
            None
        } else if let Some(value) = batch.context.as_ref() {
            match input(value) {
                Ok(value) => Some(value),
                Err(error) => {
                    return Reply::error(
                        classify(&ctx, error, &timed_out, ErrorCode::Evaluation),
                        None,
                    );
                }
            }
        } else {
            None
        };
        let scope = if batch.syntax_only {
            None
        } else if let Some(data) = batch.data.as_ref() {
            let prepare: Function = match ctx.globals().get("__scope") {
                Ok(prepare) => prepare,
                Err(_) => return Reply::error(ErrorCode::Internal, None),
            };
            let prepared = serde_json::to_vec(data)
                .ok()
                .and_then(|bytes| ctx.json_parse(bytes).ok())
                .ok_or(rquickjs::Error::Allocation)
                .and_then(|data| prepare.call::<_, rquickjs::Value>((data,)));
            match prepared {
                Ok(scope) => Some(scope),
                Err(error) => {
                    return Reply::error(
                        classify(&ctx, error, &timed_out, ErrorCode::Internal),
                        None,
                    );
                }
            }
        } else {
            None
        };
        let mut values = Vec::with_capacity(batch.evaluations.len());
        let mut output_bytes = 0usize;
        for (index, evaluation) in batch.evaluations.iter().enumerate() {
            if Instant::now() >= deadline {
                return Reply::error(ErrorCode::Timeout, Some(index));
            }
            let key = (
                evaluation.expression.as_str(),
                evaluation.template,
                evaluation.capture_presence,
            );
            let function = if let Some(function) = functions.get(&key) {
                function.clone()
            } else {
                let function = match compile.call::<_, Function>((
                    evaluation.expression.as_str(),
                    evaluation.template,
                    batch.syntax_only,
                    evaluation.capture_presence,
                )) {
                    Ok(function) => function,
                    Err(error) => {
                        return Reply::error(
                            classify(&ctx, error, &timed_out, ErrorCode::Syntax),
                            Some(index),
                        );
                    }
                };
                functions.insert(key, function.clone());
                function
            };
            if batch.syntax_only {
                continue;
            }
            let input = match shared
                .as_ref()
                .map(|value| Ok(value.clone()))
                .unwrap_or_else(|| input(&evaluation.context))
            {
                Ok(input) => input,
                Err(error) => {
                    return Reply::error(
                        classify(&ctx, error, &timed_out, ErrorCode::Evaluation),
                        Some(index),
                    );
                }
            };
            let encoded: String =
                match function.call((input, scope.as_ref(), evaluation.item_index)) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        return Reply::error(
                            classify(&ctx, error, &timed_out, ErrorCode::Evaluation),
                            Some(index),
                        );
                    }
                };
            output_bytes = match output_bytes.checked_add(encoded.len() + 1) {
                Some(bytes) if bytes <= MAX_RESPONSE_BYTES - 128 => bytes,
                _ => return Reply::error(ErrorCode::ResultLimit, Some(index)),
            };
            match serde_json::from_str(&encoded) {
                Ok(value) => values.push(value),
                Err(_) => return Reply::error(ErrorCode::Evaluation, Some(index)),
            }
        }
        Reply::Ok {
            version: VERSION,
            values,
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn frozen_dates_handle_offsets_serialization_and_reject_invalid_calendar_values() {
        use serde_json::json;
        let mut request = batch(
            "({now:$now,today:$today,delta:$now-$today,offset:$now.offset,day:$now.toISODate(),clock:typeof Date})",
        );
        request.context = Some(json!({
            "$now":{"iso":"2026-11-01T01:30:00.000-05:00","zone":"America/New_York"},
            "$today":{"iso":"2026-11-01T00:00:00.000-04:00","zone":"America/New_York"}
        }));
        request.evaluations[0].context = json!({});
        let duplicate = serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        let actual = serde_json::to_value(evaluate(duplicate)).unwrap();
        assert_eq!(
            actual["values"],
            json!([{
                "now":"2026-11-01T01:30:00.000-05:00","today":"2026-11-01T00:00:00.000-04:00",
                "delta":9000000,"offset":-300,"day":"2026-11-01","clock":"undefined"
            }])
        );
        request.context.as_mut().unwrap()["$now"]["iso"] = json!("2026-02-30T00:00:00.000Z");
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::Evaluation,
                ..
            }
        ));
    }
    use super::*;
    use ferry_protocol::sandbox::Evaluation;
    use serde_json::json;

    fn batch(source: &str) -> Batch {
        Batch {
            program: None,
            version: VERSION,
            timeout_ms: 1000,
            memory_bytes: 16 * 1024 * 1024,
            deterministic: true,
            syntax_only: false,
            data: None,
            context: None,
            evaluations: vec![Evaluation {
                expression: source.into(),
                template: false,
                capture_presence: false,
                item_index: None,
                context: json!({"$json":{"value":21},"$vars":{"suffix":"!"}}),
            }],
        }
    }
    #[test]
    fn conditional_presence_captures_only_top_level_missing_and_keeps_strict_json() {
        for (source, expected) in [
            ("$json.absent", json!({"present":false})),
            ("null", json!({"present":true,"value":null})),
            (
                "({present:false})",
                json!({"present":true,"value":{"present":false}}),
            ),
        ] {
            let mut request = batch(source);
            request.evaluations[0].capture_presence = true;
            assert!(
                matches!(evaluate(request), Reply::Ok { values,.. } if values == [expected]),
                "{source}"
            );
        }
        for source in [
            "({nested:undefined})",
            "[undefined]",
            "NaN",
            "(()=>{})",
            "Promise.resolve(1)",
        ] {
            let mut request = batch(source);
            request.evaluations[0].capture_presence = true;
            assert!(
                matches!(
                    evaluate(request),
                    Reply::Error {
                        code: ErrorCode::Evaluation,
                        ..
                    }
                ),
                "{source}"
            );
        }
        let mut request = batch("null");
        request.evaluations.push(Evaluation {
            expression: "null".into(),
            template: false,
            capture_presence: true,
            item_index: None,
            context: json!({}),
        });
        request.evaluations.push(Evaluation {
            expression: "null".into(),
            template: false,
            capture_presence: false,
            item_index: None,
            context: json!({}),
        });
        assert!(
            matches!(evaluate(request), Reply::Ok {values,..} if values == [json!(null),json!({"present":true,"value":null}),json!(null)])
        );
        let mut request = batch("=prefix {{$json.absent}}");
        request.evaluations[0].capture_presence = true;
        request.evaluations[0].template = true;
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::Evaluation,
                ..
            }
        ));
        let mut request = batch("={{$json.absent}}");
        request.evaluations[0].capture_presence = true;
        request.evaluations[0].template = true;
        assert!(
            matches!(evaluate(request), Reply::Ok {values,..} if values == [json!({"present":false})])
        );
    }

    #[test]
    fn json_types_and_context_are_preserved_without_host_globals() {
        match evaluate(batch(
            "({number:$json.value*2,text:'ok'+$vars.suffix,array:[true,null],host:[typeof process,typeof require,typeof fetch,typeof $env,typeof secrets]})",
        )) {
            Reply::Ok { values, .. } => assert_eq!(
                values,
                [
                    json!({"number":42,"text":"ok!","array":[true,null],"host":["undefined","undefined","undefined","undefined","undefined"]})
                ]
            ),
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn deterministic_evaluation_rejects_clock_random_context_and_intrinsic_mutation() {
        for source in [
            "Date.now()",
            "Math.random()",
            "($json.value=4)",
            "(Array.prototype.push=()=>0)",
            "(globalThis.other=1)",
        ] {
            assert!(
                matches!(
                    evaluate(batch(source)),
                    Reply::Error {
                        code: ErrorCode::Evaluation,
                        ..
                    }
                ),
                "{source}"
            );
        }
        assert!(
            matches!(evaluate(batch("typeof other")),Reply::Ok {values,..} if values==[json!("undefined")])
        );
    }
    #[test]
    fn syntax_only_never_executes_expressions_or_wrapper_escape_attempts() {
        for source in [
            "(()=>{while(true){}})()",
            "(()=>{throw new Error('PRIVATE')})()",
            "(globalThis.leak=1)",
            "missing.variable",
        ] {
            let mut request = batch(source);
            request.syntax_only = true;
            request.timeout_ms = 30;
            assert!(
                matches!(evaluate(request), Reply::Ok { values, .. } if values.is_empty()),
                "{source}"
            );
        }
        let mut request = batch("0)); }); (()=>{while(true){}})(); (function(){return ((0");
        request.syntax_only = true;
        request.timeout_ms = 30;
        let reply = evaluate(request);
        assert!(
            matches!(
                reply,
                Reply::Error {
                    code: ErrorCode::Syntax,
                    ..
                }
            ),
            "{reply:?}"
        );
    }
    #[test]
    fn ferry_templates_preserve_types_and_parse_js_delimiters_without_a_regex_lexer() {
        for (source, expected) in [
            ("=$json.value*2", json!(42)),
            ("={{ $json.value*2 }}", json!(42)),
            ("={{{a:1}}}", json!({"a":1})),
            ("=answer={{$json.value*2}}!", json!("answer=42!")),
            ("={{ '}}' }}", json!("}}")),
            ("={{ /}}/.test('}}') }}", json!(true)),
            ("={{ `x${({a:1}).a}` }}", json!("x1")),
            ("={{ 1 // ignored }}\n + 2 }}", json!(3)),
            (
                "=数据={{({a:[true,null]})}}",
                json!("数据={\"a\":[true,null]}"),
            ),
        ] {
            let mut request = batch(source);
            request.evaluations[0].template = true;
            match evaluate(request) {
                Reply::Ok { values, .. } => assert_eq!(values, vec![expected], "{source}"),
                other => panic!("{source}: {other:?}"),
            }
        }
        for source in [
            "={{1+}}",
            "={{}}",
            "={{'unterminated}}",
            "=text {{ Promise.resolve(1) }}",
        ] {
            let mut request = batch(source);
            request.evaluations[0].template = true;
            assert!(matches!(evaluate(request), Reply::Error { .. }), "{source}");
        }
    }

    fn lineage(source: &str) -> Batch {
        let mut request = batch(source);
        let hash = "a".repeat(64);
        let other = "b".repeat(64);
        request.data = Some(serde_json::from_value(json!({
            "items":[{"json":{"value":22}, "binary":{"file":{
                "asset":format!("sha256:{hash}"),"mime":"text/plain","name":"input.txt","size":3
            }},"paired":[{"node":"middle","iteration_hash":hash,"index":0}]}],
            "upstream":[
                {"node":"source","name":"Source","iteration_hash":hash,
                    "items":[{"json":{"value":1}},{"json":{"value":2}}]},
                {"node":"source","name":"Source","iteration_hash":other,
                    "items":[{"json":{"value":99}}]},
                {"node":"middle","name":"Middle","iteration_hash":hash,
                    "items":[{"json":{"value":20},"paired":[{"node":"source","iteration_hash":hash,"index":1}]}]}
            ]
        })).unwrap());
        request.evaluations[0].item_index = Some(0);
        request
    }

    #[test]
    fn input_helpers_and_named_lineage_use_the_selected_item_and_exact_iteration() {
        let request = lineage(
            "({value:$json.value,binary:$binary.file.name,input:$input.item.json.value,first:$input.first().json.value,count:$input.all().length,source:$('Source').item.json.value,all:$('Source').all().map(x=>x.json.value),oldest:$('Source').first().json.value})",
        );
        assert!(request.valid());
        assert!(
            matches!(evaluate(request), Reply::Ok {values,..} if values == [json!({
                "value":22,"binary":"input.txt","input":22,"first":22,"count":1,"source":2,"all":[1,2,99],"oldest":1
            })])
        );
        let mut request = lineage("$input.all().length");
        request.evaluations[0].item_index = None;
        assert!(matches!(evaluate(request), Reply::Ok {values,..} if values == [json!(1)]));
    }

    #[test]
    fn lineage_deduplicates_diamonds_and_rejects_ambiguous_or_missing_items() {
        let mut request = lineage("$('Source').item.json.value");
        let pair = &mut request.data.as_mut().unwrap().upstream[2].items[0].paired;
        pair.push(
            serde_json::from_value(
                json!({"node":"source","iteration_hash":"a".repeat(64),"index":1}),
            )
            .unwrap(),
        );
        let data = request.data.as_mut().unwrap();
        data.items[0].paired.push(
            serde_json::from_value(json!({
                "node":"branch","iteration_hash":"a".repeat(64),"index":0
            }))
            .unwrap(),
        );
        data.upstream.push(serde_json::from_value(json!({
            "node":"branch","name":"Branch","iteration_hash":"a".repeat(64),
            "items":[{"json":{},"paired":[{"node":"source","iteration_hash":"a".repeat(64),"index":1}]}]
        })).unwrap());
        assert!(matches!(evaluate(request), Reply::Ok {values,..} if values == [json!(2)]));
        for (index, hash, expected) in [
            (0, "a".repeat(64), ErrorCode::AmbiguousItem),
            (0, "b".repeat(64), ErrorCode::AmbiguousItem),
            (9, "a".repeat(64), ErrorCode::MissingItem),
        ] {
            let mut request = lineage("$('Source').item.json.value");
            let pair = &mut request.data.as_mut().unwrap().upstream[2].items[0].paired;
            pair.push(
                serde_json::from_value(
                    json!({"node":"source","iteration_hash":hash,"index":index}),
                )
                .unwrap(),
            );
            assert!(
                matches!(evaluate(request), Reply::Error {code,index:Some(0),..} if code == expected)
            );
        }
        let mut request = lineage("$('Source').item");
        request.data.as_mut().unwrap().upstream[2].items[0].paired[0].node = "middle".into();
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::MissingItem,
                ..
            }
        ));
        let mut request = lineage("$('Source').all()");
        request.data.as_mut().unwrap().upstream[2].name = "Source".into();
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::AmbiguousItem,
                ..
            }
        ));
        let mut request = lineage("$input.item");
        request.evaluations[0].item_index = None;
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::MissingItem,
                ..
            }
        ));
    }

    #[test]
    fn shared_snapshots_are_frozen_and_context_cannot_override_the_selected_input() {
        for source in [
            "$input.all().push({json:{}})",
            "($('Source').item.json.value=99)",
            "($binary.file.name='changed')",
            "(__scope.lookup.secret=99)",
        ] {
            let request = lineage(source);
            assert!(
                matches!(
                    evaluate(request),
                    Reply::Error {
                        code: ErrorCode::Evaluation,
                        ..
                    }
                ),
                "{source}"
            );
        }
        let mut request = lineage("$json.value");
        request.evaluations[0].context =
            json!({"$json":{"value":999},"$input":{"item":{"json":{"value":999}}}});
        assert!(matches!(evaluate(request), Reply::Ok {values,..} if values == [json!(22)]));
        let mut request = lineage("$('__proto__').item.json.value");
        for output in &mut request.data.as_mut().unwrap().upstream {
            if output.node == "source" {
                output.name = "__proto__".into();
            }
        }
        assert!(matches!(evaluate(request), Reply::Ok {values,..} if values == [json!(2)]));
    }

    #[test]
    fn shared_metadata_is_frozen_once_and_reused_by_every_evaluation() {
        let mut request = batch("$vars.nested.factor * $json.value");
        request.context = Some(json!({"$json":{"value":3},"$vars":{"nested":{"factor":7}}}));
        request.evaluations[0].context = json!({});
        request.evaluations.push(Evaluation {
            expression: "$vars.nested.factor * $json.value".into(),
            template: false,
            capture_presence: false,
            item_index: None,
            context: json!({}),
        });
        assert!(request.valid());
        assert!(
            matches!(evaluate(request),Reply::Ok {values,..} if values == [json!(21),json!(21)])
        );
        let mut request = batch("($vars.nested.factor=99)");
        request.context = Some(json!({"$vars":{"nested":{"factor":7}}}));
        request.evaluations[0].context = json!({});
        assert!(matches!(
            evaluate(request),
            Reply::Error {
                code: ErrorCode::Evaluation,
                ..
            }
        ));
    }
    #[test]
    fn nested_json_contexts_stay_frozen_after_the_batch_fast_path() {
        for source in [
            "($json.nested.items[0].value=99)",
            "$json.nested.items.push({value:99})",
            "Object.defineProperty($json.nested,'other',{value:99})",
            "($vars.nested.label='changed')",
        ] {
            let mut request = batch(source);
            request.evaluations[0].context = json!({
                "$json":{"nested":{"items":[{"value":21}]}},
                "$vars":{"nested":{"label":"original"}}
            });
            assert!(
                matches!(
                    evaluate(request),
                    Reply::Error {
                        code: ErrorCode::Evaluation,
                        index: Some(0),
                        ..
                    }
                ),
                "{source}"
            );
        }
    }
    #[test]
    fn syntax_and_non_json_results_are_errors_without_partial_output_or_user_exception_text() {
        assert!(matches!(
            evaluate(batch("1+")),
            Reply::Error {
                code: ErrorCode::Syntax,
                index: Some(0),
                ..
            }
        ));
        for source in [
            "NaN",
            "Infinity",
            "undefined",
            "1n",
            "(()=>{})",
            "Promise.resolve(1)",
            "(()=>{throw new Error('PRIVATE_USER_PAYLOAD')})()",
        ] {
            let mut request = batch(source);
            request.evaluations.insert(
                0,
                Evaluation {
                    expression: "42".into(),
                    template: false,
                    capture_presence: false,
                    item_index: None,
                    context: json!({}),
                },
            );
            let reply = evaluate(request);
            assert!(
                matches!(
                    reply,
                    Reply::Error {
                        code: ErrorCode::Evaluation,
                        index: Some(1),
                        ..
                    }
                ),
                "{source}: {reply:?}"
            );
            assert!(
                !serde_json::to_string(&reply)
                    .unwrap()
                    .contains("PRIVATE_USER_PAYLOAD")
            );
        }
    }
}
