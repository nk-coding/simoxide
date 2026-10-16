//! `Model` operations that make new model instances: `Model::copy()` (`ModelInstanceImpl.copy`:
//! an extent of `EcoreUtil.copyAll` of the root objects, same model type and parameter) and
//! `Model::createEmptyModel()` (an empty extent of the model type).

use crate::error::VmResult;
use crate::extent::Extent;
use crate::vm::{ObjHandle, Vm};
use qvto_model::ModelStore;
use qvto_runtime::{ObjRef, TypeRef, Value};
use qvto_sema::ir::{Direction, ModelTypeId};
use qvto_types::{Type, TypeId};

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// The model type of an extent (its parameter's, or the one it was made with).
    pub(crate) fn extent_model_type(&self, x: usize) -> Option<ModelTypeId> {
        let e = &self.extents[x];
        match e.param {
            Some((module, idx)) => Some(
                self.prog.ir.modules[module.index()].model_params[idx as usize].model_type,
            ),
            None => e.model_type,
        }
    }

    /// `Model::copy()`.
    pub(crate) fn model_copy(&mut self, src: &Value) -> VmResult<Value> {
        let Value::Model(x) = src else {
            return Ok(Value::Invalid);
        };
        let x = *x as usize;
        let roots = self.extents[x].root_objects(&*self.store);
        // EcoreUtil.copyAll: every root with its contents, then the references
        let mut map: Vec<(S::Obj, S::Obj)> = Vec::new();
        let mut copies = Vec::with_capacity(roots.len());
        for r in roots {
            if r.is_local() {
                let lo = self.local[r.index() as usize].clone();
                self.local.push(lo);
                copies.push(ObjRef::local((self.local.len() - 1) as u32));
                continue;
            }
            match self.copy_rec(S::Obj::from_u32(r.index()), true, &mut map) {
                Ok(c) => copies.push(ObjRef::host(c.to_u32())),
                Err(e) => return Ok(self.swallow(crate::objects::model_error_text(&e))),
            }
        }
        self.copy_references(&map);
        let (param, direction, label) = {
            let e = &self.extents[x];
            (e.param, e.direction, e.label.clone())
        };
        let mut e = Extent::new(param, direction, label);
        e.model_type = self.extent_model_type(x);
        let mut next = self.next_extent_uri;
        e.bind_initial(self.store, copies, &mut next);
        self.next_extent_uri = next;
        self.extents.push(e);
        Ok(Value::Model((self.extents.len() - 1) as u32))
    }

    /// `Model::createEmptyModel()` on a model (its type) or a model type.
    pub(crate) fn create_empty_model(&mut self, src: &Value) -> VmResult<Value> {
        let mt = match src {
            Value::Model(x) => self.extent_model_type(*x as usize),
            Value::Type(TypeRef(t)) => match self.prog.ir.types.get(TypeId(*t)) {
                Type::ModelType(m) => Some(ModelTypeId(*m)),
                _ => None,
            },
            _ => None,
        };
        let mut e = Extent::new(None, Direction::Out, "model".into());
        e.model_type = mt;
        self.extents.push(e);
        Ok(Value::Model((self.extents.len() - 1) as u32))
    }
}
