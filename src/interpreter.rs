use crate::ast::{CallStyle, Expr, Pattern, Stmt};
use crate::runtime::{
    CallResult, CrawValue, PlainCrawValue, craw_add2, craw_call, craw_div2, craw_div_int2,
    craw_driver, craw_eq2, craw_ge2, craw_get_attr, craw_get_item, craw_gt2, craw_hcat, craw_in2,
    craw_intersection2, craw_is_truthy, craw_le2, craw_lt2, craw_mod2, craw_mul2, craw_ne2,
    craw_notin2, craw_pow2, craw_set_attr, craw_set_item, craw_sub2, craw_subset2,
    craw_superset2, craw_to2, craw_union2, craw_unwrap, craw_until2, craw_vcat,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

/// Wraps a value that is provably single-owner across the handoff to a
/// spawned generator thread (see `GEN_CHANNEL`), so it never sees concurrent
/// access despite containing non-`Send` `Rc`/`RefCell` data.
struct ForceSend<T>(T);
unsafe impl<T> Send for ForceSend<T> {}

// Set only inside a generator's dedicated OS thread (see `Stmt::FunctionDef`
// with `is_generator`), so `Stmt::Yield` can find its handshake channels
// without threading them through every `exec_stmt`/`eval_expr` call.
thread_local! {
    static GEN_CHANNEL: RefCell<Option<(mpsc::Sender<PlainCrawValue>, mpsc::Receiver<()>)>> =
        const { RefCell::new(None) };
}

type GeneratorPayload = ForceSend<(
    Scope,
    Vec<Stmt>,
    mpsc::Sender<PlainCrawValue>,
    mpsc::Receiver<()>,
)>;

// Taking `ForceSend<T>` by value (rather than destructuring a captured
// variable inline inside the spawned closure) keeps rustc's 2021 precise
// closure captures from reaching past the wrapper into its non-`Send` field.
fn run_generator_body(payload: GeneratorPayload) {
    let ForceSend((mut local_scope, body, v_tx, c_rx)) = payload;
    GEN_CHANNEL.with(|cell| *cell.borrow_mut() = Some((v_tx, c_rx)));
    for s in &body {
        match exec_stmt(s, &mut local_scope) {
            Ok(ControlFlow::Return(_)) | Ok(ControlFlow::Break) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    GEN_CHANNEL.with(|cell| *cell.borrow_mut() = None);
}

#[derive(Clone)]
pub struct Scope {
    pub globals: Rc<RefCell<HashMap<String, CrawValue>>>,
    pub locals: Vec<HashMap<String, CrawValue>>,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            globals: Rc::new(RefCell::new(HashMap::new())),
            locals: vec![HashMap::new()],
        }
    }

    pub fn get(&self, name: &str) -> Option<CrawValue> {
        for frame in self.locals.iter().rev() {
            if let Some(val) = frame.get(name) {
                return Some(val.clone());
            }
        }
        self.globals.borrow().get(name).cloned()
    }

    pub fn set(&mut self, name: String, val: CrawValue) {
        if let Some(frame) = self.locals.last_mut() {
            frame.insert(name, val);
        } else {
            self.globals.borrow_mut().insert(name, val);
        }
    }

    pub fn push_frame(&mut self) {
        self.locals.push(HashMap::new());
    }

    pub fn pop_frame(&mut self) {
        self.locals.pop();
    }
}

fn deep_clone_value(val: &CrawValue) -> CrawValue {
    match val {
        CrawValue::List(items) => {
            let cloned: Vec<CrawValue> = items.borrow().iter().map(deep_clone_value).collect();
            CrawValue::List(Rc::new(RefCell::new(cloned)))
        }
        CrawValue::Dict(map) => {
            let cloned: HashMap<CrawValue, CrawValue> = map
                .borrow()
                .iter()
                .map(|(k, v)| (deep_clone_value(k), deep_clone_value(v)))
                .collect();
            CrawValue::Dict(Rc::new(RefCell::new(cloned)))
        }
        CrawValue::Set(items) => {
            let cloned: HashSet<CrawValue> = items.borrow().iter().map(deep_clone_value).collect();
            CrawValue::Set(Rc::new(RefCell::new(cloned)))
        }
        CrawValue::Tuple(items) => {
            let cloned: Vec<CrawValue> = items.iter().map(deep_clone_value).collect();
            CrawValue::Tuple(Rc::new(cloned))
        }
        CrawValue::Data(name, fields, values) => {
            let cloned: Vec<CrawValue> = values.borrow().iter().map(deep_clone_value).collect();
            CrawValue::Data(name.clone(), fields.clone(), Rc::new(RefCell::new(cloned)))
        }
        other => other.clone(),
    }
}

fn deep_clone_locals(
    locals: &[HashMap<String, CrawValue>],
) -> Vec<HashMap<String, CrawValue>> {
    locals
        .iter()
        .map(|frame| {
            frame
                .iter()
                .map(|(k, v)| (k.clone(), deep_clone_value(v)))
                .collect()
        })
        .collect()
}

pub enum ControlFlow {
    None,
    Return(CrawValue),
    Break,
    Continue,
}

pub fn eval_expr(expr: &Expr, scope: &mut Scope) -> Result<CrawValue, String> {
    match expr {
        Expr::Number(n) => Ok(CrawValue::Int(*n)),
        Expr::Float(f) => Ok(CrawValue::Float(*f)),
        Expr::String(s) => Ok(CrawValue::String(Rc::new(s.clone()))),
        Expr::Bool(b) => Ok(CrawValue::Bool(*b)),
        Expr::None => Ok(CrawValue::None),
        Expr::Ident(name) => scope
            .get(name)
            .ok_or_else(|| format!("Undefined variable: {}", name)),
        Expr::BinaryOp(left, op, right) => {
            let l = eval_expr(left, scope)?;
            let r = eval_expr(right, scope)?;
            match op.as_str() {
                "+" => Ok(craw_add2(l, r)),
                "-" => Ok(craw_sub2(l, r)),
                "*" => Ok(craw_mul2(l, r)),
                "/" => Ok(craw_div2(l, r)),
                "÷" => Ok(craw_div_int2(l, r)),
                "%" => Ok(craw_mod2(l, r)),
                "**" => Ok(craw_pow2(l, r)),
                "==" => Ok(craw_eq2(l, r)),
                "!=" | "≠" => Ok(craw_ne2(l, r)),
                "<" => Ok(craw_lt2(l, r)),
                "<=" | "≤" => Ok(craw_le2(l, r)),
                ">" => Ok(craw_gt2(l, r)),
                ">=" | "≥" => Ok(craw_ge2(l, r)),
                "in" | "∈" => Ok(craw_in2(l, r)),
                "notin" | "∉" => Ok(craw_notin2(l, r)),
                "|" | "∪" => Ok(craw_union2(l, r)),
                "&" | "∩" => Ok(craw_intersection2(l, r)),
                "⊆" => Ok(craw_subset2(l, r)),
                "⊇" => Ok(craw_superset2(l, r)),
                "to" => Ok(craw_to2(l, r)),
                "until" => Ok(craw_until2(l, r)),
                _ => Err(format!("Unknown operator: {}", op)),
            }
        }
        Expr::Call(target, args) => {
            let target_val = eval_expr(target, scope)?;
            let mut arg_vals = Vec::new();
            for arg in args {
                if let Expr::Splat(inner) = arg {
                    match eval_expr(inner, scope)? {
                        CrawValue::List(items) => arg_vals.extend(items.borrow().iter().cloned()),
                        _ => return Err("TypeError: splat expects a list".to_string()),
                    }
                } else {
                    arg_vals.push(eval_expr(arg, scope)?);
                }
            }
            match craw_call(target_val, arg_vals) {
                CallResult::Return(v) => Ok(v),
                _ => Err("Call failed".to_string()),
            }
        }
        Expr::BroadcastCall(target, args) => {
            let target_val = eval_expr(target, scope)?;
            let mut arg_vals = Vec::new();
            for arg in args {
                arg_vals.push(eval_expr(arg, scope)?);
            }
            match crate::runtime::craw_broadcast(target_val, arg_vals) {
                CallResult::Return(v) => Ok(v),
                _ => Err("Broadcast call failed".to_string()),
            }
        }
        Expr::Lambda(params, body, _) => {
            let body = body.clone();
            let params = params.clone();
            let globals = scope.globals.clone();
            let current_locals = scope.locals.clone();

            let closure = move |actual_args: Vec<CrawValue>| {
                let mut local_scope = Scope {
                    globals: globals.clone(),
                    locals: current_locals.clone(),
                };
                local_scope.push_frame();
                for (i, param_name) in params.iter().enumerate() {
                    if i < actual_args.len() {
                        local_scope.set(param_name.clone(), actual_args[i].clone());
                    }
                }
                match eval_expr(&body, &mut local_scope) {
                    Ok(v) => CallResult::Return(v),
                    Err(e) => panic!("Runtime error in lambda: {}", e),
                }
            };

            Ok(CrawValue::Closure(Rc::new(closure)))
        }
        Expr::ImplicitLambda(body, _id) => {
            let body = body.clone();
            let globals = scope.globals.clone();
            let current_locals = scope.locals.clone();

            let closure = move |actual_args: Vec<CrawValue>| {
                let mut local_scope = Scope {
                    globals: globals.clone(),
                    locals: current_locals.clone(),
                };
                local_scope.push_frame();
                local_scope.set("_".to_string(), actual_args[0].clone());
                match eval_expr(&body, &mut local_scope) {
                    Ok(v) => CallResult::Return(v),
                    Err(e) => panic!("Runtime error in implicit lambda: {}", e),
                }
            };

            Ok(CrawValue::Closure(Rc::new(closure)))
        }
        Expr::List(items) => {
            let mut vals = Vec::new();
            for item in items {
                vals.push(eval_expr(item, scope)?);
            }
            Ok(CrawValue::List(Rc::new(RefCell::new(vals))))
        }
        Expr::Set(items) => {
            let mut vals = HashSet::new();
            for item in items {
                vals.insert(eval_expr(item, scope)?);
            }
            Ok(CrawValue::Set(Rc::new(RefCell::new(vals))))
        }
        Expr::Tuple(items) => {
            let mut vals = Vec::new();
            for item in items {
                vals.push(eval_expr(item, scope)?);
            }
            Ok(CrawValue::Tuple(Rc::new(vals)))
        }
        Expr::Dict(items) => {
            let mut vals = HashMap::new();
            for (k, v) in items {
                vals.insert(eval_expr(k, scope)?, eval_expr(v, scope)?);
            }
            Ok(CrawValue::Dict(Rc::new(RefCell::new(vals))))
        }
        Expr::Where(expr, stmts, _) => {
            scope.push_frame();
            let res = (|| -> Result<CrawValue, String> {
                for s in stmts {
                    exec_stmt(s, scope)?;
                }
                eval_expr(expr, scope)
            })();
            scope.pop_frame();
            res
        }
        Expr::Attribute(obj, attr) => {
            let obj_val = eval_expr(obj, scope)?;
            match craw_get_attr(obj_val, attr) {
                CallResult::Return(v) => Ok(v),
                _ => Err(format!("Attribute access failed: {}", attr)),
            }
        }
        Expr::Index(obj, idx) => {
            let obj_val = eval_expr(obj, scope)?;
            let idx_val = eval_expr(idx, scope)?;
            match craw_get_item(obj_val, idx_val) {
                CallResult::Return(v) => Ok(v),
                _ => Err("Index access failed".to_string()),
            }
        }
        Expr::Ternary(cond, then_expr, else_expr) => {
            if craw_is_truthy(&eval_expr(cond, scope)?) {
                eval_expr(then_expr, scope)
            } else {
                eval_expr(else_expr, scope)
            }
        }
        Expr::Range(start, end) => {
            let s = eval_expr(start, scope)?;
            let e = eval_expr(end, scope)?;
            Ok(craw_to2(s, e))
        }
        Expr::Slice(start, stop, step) => {
            let to_i64 = |e: &Option<Box<Expr>>, scope: &mut Scope| -> Result<Option<i64>, String> {
                match e {
                    None => Ok(None),
                    Some(inner) => match eval_expr(inner, scope)? {
                        CrawValue::Int(n) => Ok(Some(n)),
                        _ => Err("TypeError: slice bound must be an int".to_string()),
                    },
                }
            };
            let s = to_i64(start, scope)?;
            let e = to_i64(stop, scope)?;
            let st = to_i64(step, scope)?;
            Ok(CrawValue::Slice(s, e, st))
        }
        Expr::NoneCoalesce(left, right) => {
            let l = eval_expr(left, scope)?;
            if matches!(l, CrawValue::None) {
                eval_expr(right, scope)
            } else {
                Ok(l)
            }
        }
        Expr::Splat(inner) => eval_expr(inner, scope),
        Expr::Gather(coll, field) => {
            let items = match eval_expr(coll, scope)? {
                CrawValue::List(items) => items.borrow().clone(),
                _ => return Err("TypeError: gather expects a list".to_string()),
            };
            let mut results = Vec::new();
            for item in items {
                match craw_get_attr(item, field) {
                    CallResult::Return(v) => results.push(v),
                    _ => return Err(format!("Attribute access failed: {}", field)),
                }
            }
            Ok(CrawValue::List(Rc::new(RefCell::new(results))))
        }
        Expr::FString(exprs) => {
            let mut out = String::new();
            for e in exprs {
                if let Expr::String(s) = e {
                    out.push_str(s);
                } else {
                    out.push_str(&eval_expr(e, scope)?.to_string());
                }
            }
            Ok(CrawValue::String(Rc::new(out)))
        }
        Expr::PartialCall(target, args, _id) => {
            let target_val = eval_expr(target, scope)?;
            let mut bound: Vec<Option<CrawValue>> = Vec::with_capacity(args.len());
            for arg in args {
                match arg {
                    Some(e) => bound.push(Some(eval_expr(e, scope)?)),
                    None => bound.push(None),
                }
            }
            let closure = move |extra_args: Vec<CrawValue>| {
                let mut final_args = Vec::new();
                let mut extra_idx = 0;
                for slot in &bound {
                    match slot {
                        Some(v) => final_args.push(v.clone()),
                        None => {
                            final_args.push(extra_args[extra_idx].clone());
                            extra_idx += 1;
                        }
                    }
                }
                while extra_idx < extra_args.len() {
                    final_args.push(extra_args[extra_idx].clone());
                    extra_idx += 1;
                }
                craw_call(target_val.clone(), final_args)
            };
            Ok(CrawValue::Closure(Rc::new(closure)))
        }
        Expr::Comprehension(expr, pat, iterable, is_lazy, _id, filter) => {
            if *is_lazy {
                return Err("Interpreter does not yet support lazy comprehensions".to_string());
            }
            let iter_val = craw_unwrap(eval_expr(iterable, scope)?);
            let items = match iter_val {
                CrawValue::List(items) => items.borrow().clone(),
                _ => return Err("TypeError: expected iterable in comprehension".to_string()),
            };
            let mut results = Vec::new();
            for item in items {
                scope.push_frame();
                let result = (|| -> Result<Option<CrawValue>, String> {
                    if !bind_pattern(pat, &item, scope)? {
                        return Ok(None);
                    }
                    if let Some(filter_expr) = filter
                        && !craw_is_truthy(&eval_expr(filter_expr, scope)?)
                    {
                        return Ok(None);
                    }
                    eval_expr(expr, scope).map(Some)
                })();
                scope.pop_frame();
                if let Some(v) = result? {
                    results.push(v);
                }
            }
            Ok(CrawValue::List(Rc::new(RefCell::new(results))))
        }
        Expr::Pipe(left, data, right) => {
            let l = eval_expr(left, scope)?;
            if data.none_aware && matches!(l, CrawValue::None) {
                return Ok(CrawValue::None);
            }
            let right_val = eval_expr(right, scope)?;
            let args = match data.style {
                CallStyle::Standard => vec![l],
                CallStyle::Star => match l {
                    CrawValue::List(items) => items.borrow().clone(),
                    _ => return Err("TypeError: star pipe expects a list".to_string()),
                },
                CallStyle::DoubleStar => match l {
                    CrawValue::Dict(items) => items.borrow().values().cloned().collect(),
                    _ => return Err("TypeError: double star pipe expects a dict".to_string()),
                },
            };
            match craw_call(right_val, args) {
                CallResult::Return(v) => Ok(v),
                _ => Err("Pipe call failed".to_string()),
            }
        }
        Expr::Hcat(items) => {
            let mut vals = Vec::new();
            for item in items {
                vals.push(eval_expr(item, scope)?);
            }
            Ok(craw_hcat(vals))
        }
        Expr::Vcat(items) => {
            let mut vals = Vec::new();
            for item in items {
                vals.push(eval_expr(item, scope)?);
            }
            Ok(craw_vcat(vals))
        }
        // Inline Rust blocks are a compile-time/transpiler-only feature (there
        // is no Rust compiler embedded in the interpreter), matching how
        // `Stmt::TemplateDef`/`MacroDef` are already treated below. A numeric
        // placeholder (rather than None) keeps arithmetic on the result
        // (e.g. measuring elapsed time around a passthrough block) from
        // hard-panicking under the interpreter.
        Expr::Passthrough(_) => Ok(CrawValue::Float(0.0)),
        Expr::Compose(left, _data, right, _id) => {
            let left_val = eval_expr(left, scope)?;
            let right_val = eval_expr(right, scope)?;
            let closure = move |args: Vec<CrawValue>| {
                let inner = craw_driver(craw_call(right_val.clone(), args));
                craw_call(left_val.clone(), vec![inner])
            };
            Ok(CrawValue::Closure(Rc::new(closure)))
        }
        _ => Err(format!(
            "Interpreter does not yet support expression: {:?}",
            expr
        )),
    }
}

pub fn exec_stmt(stmt: &Stmt, scope: &mut Scope) -> Result<ControlFlow, String> {
    match stmt {
        Stmt::Expr(expr) => {
            eval_expr(expr, scope)?;
            Ok(ControlFlow::None)
        }
        Stmt::Assign(pattern, expr) => {
            let val = eval_expr(expr, scope)?;
            if !bind_pattern(pattern, &val, scope)? {
                return Err(format!("Pattern match failed: {:?}", pattern));
            }
            Ok(ControlFlow::None)
        }
        Stmt::If(cond, then_body) => {
            if craw_is_truthy(&eval_expr(cond, scope)?) {
                for s in then_body {
                    let res = exec_stmt(s, scope)?;
                    if !matches!(res, ControlFlow::None) {
                        return Ok(res);
                    }
                }
            }
            Ok(ControlFlow::None)
        }
        Stmt::While(cond, body) => {
            while craw_is_truthy(&eval_expr(cond, scope)?) {
                for s in body {
                    let res = exec_stmt(s, scope)?;
                    match res {
                        ControlFlow::Break => return Ok(ControlFlow::None),
                        ControlFlow::Continue => break,
                        ControlFlow::Return(_) => return Ok(res),
                        ControlFlow::None => {}
                    }
                }
            }
            Ok(ControlFlow::None)
        }
        Stmt::Break => Ok(ControlFlow::Break),
        Stmt::Return(expr) => Ok(ControlFlow::Return(eval_expr(expr, scope)?)),
        Stmt::Match(expr, cases) => {
            let match_val = eval_expr(expr, scope)?;
            for (pat, guard, body) in cases {
                if !bind_pattern(pat, &match_val, scope)? {
                    continue;
                }
                if let Some(g) = guard
                    && !craw_is_truthy(&eval_expr(g, scope)?)
                {
                    continue;
                }
                for s in body {
                    let res = exec_stmt(s, scope)?;
                    if !matches!(res, ControlFlow::None) {
                        return Ok(res);
                    }
                }
                return Ok(ControlFlow::None);
            }
            Ok(ControlFlow::None)
        }
        Stmt::MatchFor(pat, expr, body) => {
            let iter_val = eval_expr(expr, scope)?;
            let items = match iter_val {
                CrawValue::List(items) => items.borrow().clone(),
                _ => return Err("TypeError: expected list for loop".to_string()),
            };
            for item in items {
                if !bind_pattern(pat, &item, scope)? {
                    return Err("MatchError: loop item did not match pattern".to_string());
                }
                let mut should_break = false;
                for s in body {
                    match exec_stmt(s, scope)? {
                        ControlFlow::Break => {
                            should_break = true;
                            break;
                        }
                        ControlFlow::Continue => break,
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::None => {}
                    }
                }
                if should_break {
                    break;
                }
            }
            Ok(ControlFlow::None)
        }
        Stmt::IndexAssign(base, index, rhs) => {
            let base_val = eval_expr(base, scope)?;
            let index_val = eval_expr(index, scope)?;
            let rhs_val = eval_expr(rhs, scope)?;
            craw_set_item(base_val, index_val, rhs_val);
            Ok(ControlFlow::None)
        }
        Stmt::AttributeAssign(base, attr, rhs) => {
            let base_val = eval_expr(base, scope)?;
            let rhs_val = eval_expr(rhs, scope)?;
            match craw_set_attr(base_val, attr, rhs_val) {
                CallResult::Return(_) => Ok(ControlFlow::None),
                _ => Err(format!("Attribute assignment failed: {}", attr)),
            }
        }
        Stmt::FunctionDef {
            name,
            args,
            vararg,
            body,
            is_copyclosure,
            is_generator,
            ..
        } => {
            let name = name.last().unwrap().clone();
            let body = body.clone();
            let args = args.clone();
            let vararg = vararg.clone();
            let globals = scope.globals.clone();
            let current_locals = if *is_copyclosure {
                deep_clone_locals(&scope.locals)
            } else {
                scope.locals.clone()
            };
            let is_generator = *is_generator;

            let closure = move |actual_args: Vec<CrawValue>| {
                let mut local_scope = Scope {
                    globals: globals.clone(),
                    locals: current_locals.clone(),
                };
                local_scope.push_frame();
                for (i, (arg_pat, default)) in args.iter().enumerate() {
                    if i < actual_args.len() {
                        if let Err(e) = bind_pattern(arg_pat, &actual_args[i], &mut local_scope) {
                            panic!("Runtime error in pattern binding: {}", e);
                        }
                    } else if let Some(default_expr) = default {
                        let default_val = match eval_expr(default_expr, &mut local_scope) {
                            Ok(v) => v,
                            Err(e) => panic!("Runtime error evaluating default argument: {}", e),
                        };
                        if let Err(e) = bind_pattern(arg_pat, &default_val, &mut local_scope) {
                            panic!("Runtime error in pattern binding: {}", e);
                        }
                    }
                }
                if let Some(vararg_name) = &vararg {
                    let extra = if actual_args.len() > args.len() {
                        actual_args[args.len()..].to_vec()
                    } else {
                        Vec::new()
                    };
                    local_scope.set(
                        vararg_name.clone(),
                        CrawValue::List(Rc::new(RefCell::new(extra))),
                    );
                }

                if is_generator {
                    let (v_tx, v_rx) = mpsc::channel::<PlainCrawValue>();
                    let (c_tx, c_rx) = mpsc::channel::<()>();
                    let payload = ForceSend((local_scope, body.clone(), v_tx, c_rx));
                    std::thread::spawn(move || run_generator_body(payload));
                    return CallResult::Return(CrawValue::Generator(
                        Arc::new(Mutex::new(v_rx)),
                        Arc::new(Mutex::new(c_tx)),
                    ));
                }

                for s in &body {
                    match exec_stmt(s, &mut local_scope) {
                        Ok(ControlFlow::Return(v)) => return CallResult::Return(v),
                        Ok(ControlFlow::Break) => break,
                        Ok(ControlFlow::Continue) => continue,
                        Ok(_) => {}
                        Err(e) => panic!("Runtime error in closure: {}", e),
                    }
                }
                CallResult::Return(CrawValue::None)
            };

            scope.set(name, CrawValue::Closure(Rc::new(closure)));
            Ok(ControlFlow::None)
        }
        Stmt::DataDef(name, fields, _) => {
            let name_clone = name.clone();
            let field_names: Vec<String> = fields.iter().map(|(f, _, _)| f.clone()).collect();

            let constructor = move |args: Vec<CrawValue>| {
                CallResult::Return(CrawValue::Data(
                    name_clone.clone(),
                    field_names.clone(),
                    Rc::new(RefCell::new(args)),
                ))
            };

            scope.set(name.clone(), CrawValue::Closure(Rc::new(constructor)));
            Ok(ControlFlow::None)
        }
        Stmt::TemplateDef(_, _, _, _) | Stmt::MacroDef { .. } => {
            // Templates and macros are compile-time/transpiler features
            Ok(ControlFlow::None)
        }
        Stmt::Passthrough(_) | Stmt::Use(_) => Ok(ControlFlow::None),
        Stmt::Yield(expr) => {
            let val = eval_expr(expr, scope)?;
            let plain = val.to_plain();
            let stopped = GEN_CHANNEL.with(|cell| {
                let mut opt = cell.borrow_mut();
                match opt.as_mut() {
                    Some((tx, rx)) => rx.recv().is_err() || tx.send(plain).is_err(),
                    None => true,
                }
            });
            if stopped {
                return Err("__generator_stopped__".to_string());
            }
            Ok(ControlFlow::None)
        }
        Stmt::Nonlocal(_) | Stmt::Global(_) => Ok(ControlFlow::None),
        _ => Err(format!(
            "Interpreter does not yet support statement: {:?}",
            stmt
        )),
    }
}

pub fn bind_pattern(
    pattern: &Pattern,
    value: &CrawValue,
    scope: &mut Scope,
) -> Result<bool, String> {
    match pattern {
        Pattern::Var(name, _) => {
            scope.set(name.clone(), value.clone());
            Ok(true)
        }
        Pattern::Wildcard => Ok(true),
        Pattern::Data(name, field_pats) => match value {
            CrawValue::Data(vname, _, fields) if vname == name => {
                let fields_ref = fields.borrow();
                if fields_ref.len() != field_pats.len() {
                    return Ok(false);
                }
                for (i, p) in field_pats.iter().enumerate() {
                    if !bind_pattern(p, &fields_ref[i], scope)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            CrawValue::List(items) if name == "List" => {
                let items_ref = items.borrow();
                let mut has_rest = false;
                let mut rest_idx = 0;
                for (i, p) in field_pats.iter().enumerate() {
                    if let Pattern::Rest(_) = p {
                        has_rest = true;
                        rest_idx = i;
                        break;
                    }
                }

                if has_rest {
                    if items_ref.len() < field_pats.len() - 1 {
                        return Ok(false);
                    }
                    for i in 0..rest_idx {
                        if !bind_pattern(&field_pats[i], &items_ref[i], scope)? {
                            return Ok(false);
                        }
                    }
                    if let Pattern::Rest(rest_var_name) = &field_pats[rest_idx] {
                        let end_idx = items_ref.len() - (field_pats.len() - 1 - rest_idx);
                        let rest_items = items_ref[rest_idx..end_idx].to_vec();
                        scope.set(
                            rest_var_name.clone(),
                            CrawValue::List(Rc::new(RefCell::new(rest_items))),
                        );
                    }
                    for i in (rest_idx + 1)..field_pats.len() {
                        let actual_idx = items_ref.len() - (field_pats.len() - i);
                        if !bind_pattern(&field_pats[i], &items_ref[actual_idx], scope)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    if items_ref.len() != field_pats.len() {
                        return Ok(false);
                    }
                    for (i, p) in field_pats.iter().enumerate() {
                        if !bind_pattern(p, &items_ref[i], scope)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
            _ => Ok(false),
        },
        Pattern::Tuple(pats) => match value {
            CrawValue::Tuple(items) => {
                let mut has_rest = false;
                let mut rest_idx = 0;
                for (i, p) in pats.iter().enumerate() {
                    if let Pattern::Rest(_) = p {
                        has_rest = true;
                        rest_idx = i;
                        break;
                    }
                }

                if has_rest {
                    if items.len() < pats.len() - 1 {
                        return Ok(false);
                    }
                    for i in 0..rest_idx {
                        if !bind_pattern(&pats[i], &items[i], scope)? {
                            return Ok(false);
                        }
                    }
                    if let Pattern::Rest(rest_var_name) = &pats[rest_idx] {
                        let end_idx = items.len() - (pats.len() - 1 - rest_idx);
                        let rest_items = items[rest_idx..end_idx].to_vec();
                        scope.set(rest_var_name.clone(), CrawValue::Tuple(Rc::new(rest_items)));
                    }
                    for i in (rest_idx + 1)..pats.len() {
                        let actual_idx = items.len() - (pats.len() - i);
                        if !bind_pattern(&pats[i], &items[actual_idx], scope)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    if items.len() != pats.len() {
                        return Ok(false);
                    }
                    for (i, p) in pats.iter().enumerate() {
                        if !bind_pattern(p, &items[i], scope)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
            _ => Ok(false),
        },
        Pattern::Const(expr) => {
            let expected_val = eval_expr(expr, scope)?;
            Ok(expected_val == *value)
        }
        Pattern::StringSplit(pat_str, var_name, is_prefix) => {
            if let CrawValue::String(rc_str) = value {
                if *is_prefix {
                    if rc_str.starts_with(pat_str) {
                        let remaining = rc_str[pat_str.len()..].to_string();
                        scope.set(var_name.clone(), CrawValue::String(Rc::new(remaining)));
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                } else {
                    if rc_str.ends_with(pat_str) {
                        let remaining = rc_str[..rc_str.len() - pat_str.len()].to_string();
                        scope.set(var_name.clone(), CrawValue::String(Rc::new(remaining)));
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                }
            } else {
                Ok(false)
            }
        }
        Pattern::View(expr, sub_pat) => {
            let func_val = eval_expr(expr, scope)?;
            match craw_call(func_val, vec![value.clone()]) {
                CallResult::Return(res) => bind_pattern(sub_pat, &res, scope),
                _ => Err("View pattern function call did not return a value".to_string()),
            }
        }
        _ => Err(format!(
            "Interpreter does not yet support pattern: {:?}",
            pattern
        )),
    }
}
