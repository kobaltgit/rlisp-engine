use std::cell::RefCell;
use std::rc::Rc;

use crate::gc::Gc;
use crate::interp::builtins::{key_str, list_elems};
use crate::interp::env::env_define;
use crate::interp::value::{int_val, need_args as need, num_val, Value};

fn def<F: Fn(&[Value]) -> Result<Value, String> + 'static>(env: Gc, name: &str, f: F) {
    env_define(env, name, Value::Native(Rc::new(f), name.to_string()));
}

// — этап 9.3: векторный ALU —
// Горячие per-vertex циклы сцены (вода: 625 вершин × ~50 интерпретируемых
// операций каждый тик) переходят на пакетные операции над f32-векторами:
// десяток вызовов f32vec-op! вместо тысяч f32set!/f32ref.
// Операнд a/b — f32-вектор или число (броадкаст); длина векторов равна dst.
fn f32_operand(v: &Value, n: usize, what: &str) -> Result<Vec<f32>, String> {
    match v {
        Value::F32Vec(r) => {
            let b = r.borrow();
            if b.len() != n {
                return Err(format!(
                    "f32vec-op!: длина операнда {} = {} ≠ {} (dst)",
                    what,
                    b.len(),
                    n
                ));
            }
            Ok(b.clone())
        }
        other => {
            let s = num_val(other)? as f32;
            Ok(vec![s; n])
        }
    }
}

pub fn register_vector_builtins(env: Gc) {
    // — векторы —
    def(env, "make-vector", |args| {
        need(args, 1)?;
        let n = int_val(&args[0])?.max(0) as usize;
        let fill = args.get(1).cloned().unwrap_or(Value::Int(0));
        Ok(Value::Vector(Rc::new(RefCell::new(vec![fill; n]))))
    });
    def(env, "vector", |args| {
        Ok(Value::Vector(Rc::new(RefCell::new(args.to_vec()))))
    });
    def(env, "vref", |args| {
        need(args, 2)?;
        let i = int_val(&args[1])? as usize;
        match &args[0] {
            Value::Vector(r) => r
                .borrow()
                .get(i)
                .cloned()
                .ok_or_else(|| "vref: индекс за границей".to_string()),
            _ => Err("vref: нужен вектор".into()),
        }
    });
    def(env, "vset!", |args| {
        need(args, 3)?;
        let i = int_val(&args[1])? as usize;
        match &args[0] {
            Value::Vector(r) => {
                let mut b = r.borrow_mut();
                let n = b.len();
                b.get_mut(i)
                    .map(|slot| *slot = args[2].clone())
                    .ok_or_else(|| format!("vset!: индекс за границей (len {})", n))?;
                Ok(Value::Nil)
            }
            _ => Err("vset!: нужен вектор".into()),
        }
    });
    def(env, "vector-length", |args| {
        need(args, 1)?;
        match &args[0] {
            Value::Vector(r) => Ok(Value::Int(r.borrow().len() as i64)),
            _ => Err("vector-length: нужен вектор".into()),
        }
    });
    def(env, "vector->list", |args| {
        need(args, 1)?;
        match &args[0] {
            Value::Vector(r) => Ok(Value::List(r.borrow().clone())),
            _ => Err("vector->list: нужен вектор".into()),
        }
    });
    def(env, "list->vector", |args| {
        need(args, 1)?;
        // Этап 2: и Vec-список, и Cons-цепочка
        let elems = list_elems(&args[0], "list->vector")?;
        Ok(Value::Vector(Rc::new(RefCell::new(elems))))
    });

    // — f32-векторы (горячие данные для SIMD, см. ТЗ §4.3) —
    def(env, "make-f32vec", |args| {
        need(args, 1)?;
        let n = int_val(&args[0])?.max(0) as usize;
        let fill = match args.get(1) {
            Some(v) => num_val(v)? as f32,
            None => 0.0,
        };
        Ok(Value::F32Vec(Rc::new(RefCell::new(vec![fill; n]))))
    });
    def(env, "f32-from-list", |args| {
        need(args, 1)?;
        // Этап 2: и Vec-список, и Cons-цепочка
        let elems = list_elems(&args[0], "f32-from-list")?;
        let mut v = Vec::with_capacity(elems.len());
        for x in &elems {
            v.push(num_val(x)? as f32);
        }
        Ok(Value::F32Vec(Rc::new(RefCell::new(v))))
    });
    def(env, "f32vec-length", |args| {
        need(args, 1)?;
        match &args[0] {
            Value::F32Vec(r) => Ok(Value::Int(r.borrow().len() as i64)),
            _ => Err("f32vec-length: нужен f32-вектор".into()),
        }
    });
    def(env, "f32ref", |args| {
        need(args, 2)?;
        let i = int_val(&args[1])? as usize;
        match &args[0] {
            Value::F32Vec(r) => r
                .borrow()
                .get(i)
                .map(|x| Value::Float(*x as f64))
                .ok_or_else(|| "f32ref: индекс за границей".to_string()),
            _ => Err("f32ref: нужен f32-вектор".into()),
        }
    });
    def(env, "f32set!", |args| {
        need(args, 3)?;
        let i = int_val(&args[1])? as usize;
        let v = num_val(&args[2])? as f32;
        match &args[0] {
            Value::F32Vec(r) => {
                let mut b = r.borrow_mut();
                let n = b.len();
                b.get_mut(i)
                    .map(|slot| *slot = v)
                    .ok_or_else(|| format!("f32set!: индекс за границей (len {})", n))?;
                Ok(Value::Nil)
            }
            _ => Err("f32set!: нужен f32-вектор".into()),
        }
    });
    def(env, "f32vec->list", |args| {
        need(args, 1)?;
        match &args[0] {
            Value::F32Vec(r) => Ok(Value::List(
                r.borrow().iter().map(|x| Value::Float(*x as f64)).collect(),
            )),
            _ => Err("f32vec->list: нужен f32-вектор".into()),
        }
    });

    def(env, "f32vec-op!", |args| {
        if args.len() < 3 || args.len() > 4 {
            return Err("f32vec-op!: (f32vec-op! dst 'op a [b])".into());
        }
        let op = key_str(&args[1])?;
        let dst = match &args[0] {
            Value::F32Vec(r) => r.clone(),
            _ => return Err("f32vec-op!: dst должен быть f32-вектором".into()),
        };
        let n = dst.borrow().len();
        // оба операнда собираем ДО borrow_mut: dst может совпадать с a/b
        let a = f32_operand(&args[2], n, "a")?;
        let b = match args.get(3) {
            Some(v) => Some(f32_operand(v, n, "b")?),
            None => None,
        };
        let mut d = dst.borrow_mut();
        if d.len() != n {
            return Err("f32vec-op!: dst изменил длину".into());
        }
        match op.as_str() {
            "sqrt" => {
                for (o, x) in d.iter_mut().zip(&a) {
                    *o = x.sqrt();
                }
            }
            "abs" => {
                for (o, x) in d.iter_mut().zip(&a) {
                    *o = x.abs();
                }
            }
            "sin" => {
                for (o, x) in d.iter_mut().zip(&a) {
                    *o = x.sin();
                }
            }
            "cos" => {
                for (o, x) in d.iter_mut().zip(&a) {
                    *o = x.cos();
                }
            }
            "neg" => {
                for (o, x) in d.iter_mut().zip(&a) {
                    *o = -x;
                }
            }
            "+" | "-" | "*" | "/" | "min" | "max" => {
                let b = match &b {
                    Some(v) => v,
                    None => return Err(format!("f32vec-op!: '{}' требует операнд b", op)),
                };
                for i in 0..n {
                    d[i] = match op.as_str() {
                        "+" => a[i] + b[i],
                        "-" => a[i] - b[i],
                        "*" => a[i] * b[i],
                        "/" => a[i] / b[i],
                        "min" => a[i].min(b[i]),
                        _ => a[i].max(b[i]),
                    };
                }
            }
            _ => return Err(format!("f32vec-op!: неизвестная операция '{}'", op)),
        }
        Ok(Value::Nil)
    });

    // рассылка плотного вектора в страйд-буфер: dst[k·stride + offset] = src[k]
    // (пример: канал RGBA → пер-вершинный буфер mesh-vc со страйдом 4)
    def(env, "f32vec-strided-set!", |args| {
        need(args, 4)?;
        let stride = int_val(&args[1])? as usize;
        let off = int_val(&args[2])? as usize;
        if stride == 0 || off >= stride {
            return Err("f32vec-strided-set!: нужно stride ≥ 1 и offset < stride".into());
        }
        let src: Vec<f32> = match &args[3] {
            Value::F32Vec(r) => r.borrow().clone(),
            _ => return Err("f32vec-strided-set!: src должен быть f32-вектором".into()),
        };
        match &args[0] {
            Value::F32Vec(r) => {
                let mut d = r.borrow_mut();
                let need = if src.is_empty() {
                    0
                } else {
                    (src.len() - 1) * stride + off + 1
                };
                if d.len() < need {
                    return Err(format!(
                        "f32vec-strided-set!: dst короче: {} < {}",
                        d.len(),
                        need
                    ));
                }
                for (k, v) in src.iter().enumerate() {
                    d[k * stride + off] = *v;
                }
                Ok(Value::Nil)
            }
            _ => Err("f32vec-strided-set!: dst должен быть f32-вектором".into()),
        }
    });
}
