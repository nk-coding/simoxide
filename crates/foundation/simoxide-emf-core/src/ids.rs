//! Dense ids. Metamodel ids are unique within one [`crate::MetaRegistry`]; a registry only grows,
//! so an id never changes its meaning once handed out.

macro_rules! id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub struct $name(pub u32);

        impl $name {
            #[inline]
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id!(
    /// An `EPackage` of the registry.
    PackageId
);
id!(
    /// An `EClass` of the registry.
    ClassId
);
id!(
    /// An `EStructuralFeature` (attribute or reference) of the registry.
    FeatureId
);
id!(
    /// An `EDataType` that is not an enumeration.
    DataTypeId
);
id!(
    /// An `EEnum`.
    EnumId
);
id!(
    /// A resource (one model file or one in-memory extent) of a store.
    ResourceId
);
