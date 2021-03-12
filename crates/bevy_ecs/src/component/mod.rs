mod type_info;

pub use type_info::*;

use std::collections::HashMap;

use crate::storage::SparseSetIndex;
use bitflags::bitflags;
use std::{
    alloc::Layout,
    any::{Any, TypeId},
    collections::hash_map::Entry,
};
use thiserror::Error;

pub trait Component: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Component for T {}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StorageType {
    Table,
    SparseSet,
}

impl Default for StorageType {
    fn default() -> Self {
        StorageType::Table
    }
}

#[derive(Debug)]
pub struct DataLayout {
    name: String,
    storage_type: StorageType,
    // SAFETY: This must remain private. It must only be set to "true" if this component is actually Send + Sync
    is_send_and_sync: bool,
    type_id: Option<TypeId>,
    layout: Layout,
    drop: unsafe fn(*mut u8),
}

impl DataLayout {
    pub unsafe fn new(
        name: Option<String>,
        storage_type: StorageType,
        is_send_and_sync: bool,
        layout: Layout,
        drop: unsafe fn(*mut u8),
    ) -> Self {
        Self {
            name: name.unwrap_or(String::new()),
            storage_type,
            is_send_and_sync,
            type_id: None,
            layout,
            drop,
        }
    }

    pub fn from_generic<T: Component>(storage_type: StorageType) -> Self {
        Self {
            name: std::any::type_name::<T>().to_string(),
            storage_type,
            is_send_and_sync: true,
            type_id: Some(TypeId::of::<T>()),
            layout: Layout::new::<T>(),
            drop: TypeInfo::drop_ptr::<T>,
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn type_id(&self) -> Option<TypeId> {
        self.type_id
    }

    #[inline]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    #[inline]
    pub fn drop(&self) -> unsafe fn(*mut u8) {
        self.drop
    }

    #[inline]
    pub fn storage_type(&self) -> StorageType {
        self.storage_type
    }

    #[inline]
    pub fn is_send_and_sync(&self) -> bool {
        self.is_send_and_sync
    }
}

impl From<TypeInfo> for DataLayout {
    fn from(type_info: TypeInfo) -> Self {
        Self {
            name: type_info.type_name().to_string(),
            storage_type: StorageType::default(),
            is_send_and_sync: type_info.is_send_and_sync(),
            type_id: Some(type_info.type_id()),
            drop: type_info.drop(),
            layout: type_info.layout(),
        }
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct RelationshipId(usize);

impl RelationshipId {
    #[inline]
    pub const fn new(index: usize) -> RelationshipId {
        RelationshipId(index)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0
    }
}

impl SparseSetIndex for RelationshipId {
    #[inline]
    fn sparse_set_index(&self) -> usize {
        self.index()
    }

    fn get_sparse_set_index(value: usize) -> Self {
        Self(value)
    }
}

pub mod relationship_kinds {
    pub struct HasComponent;
    pub struct HasResource;
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub enum EntityOrDummyId {
    Entity(crate::entity::Entity),
    /// DummyId is used so that we can fit regular components into the relationship model i.e.
    /// HasComponent Position
    /// however we dont want to implicitly spawn entities for use as the relationship target
    /// so we use a dummy id
    DummyId(DummyId),
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct Relship {
    kind: RelationshipKindId,
    target: EntityOrDummyId,
}

impl Relship {
    pub fn new(kind: RelationshipKindId, target: EntityOrDummyId) -> Self {
        Self { kind, target }
    }
}

#[derive(Debug)]
pub struct RelationshipInfo {
    id: RelationshipId,
    relationship: Relship,
    data: DataLayout,
}

impl RelationshipInfo {
    pub fn id(&self) -> RelationshipId {
        self.id
    }

    pub fn data_layout(&self) -> &DataLayout {
        &self.data
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct RelationshipKindId(usize);
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct RelationshipKindInfo {
    // TODO(Boxy) eventually we will have actual data but for now not so much
    id: RelationshipKindId,
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct DummyInfo {
    rust_type: Option<TypeId>,
    id: DummyId,
}
#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq)]
pub struct DummyId(usize);

#[derive(Debug)]
pub struct Relationships {
    relationships: Vec<RelationshipInfo>,
    relationship_indices: HashMap<Relship, RelationshipId, fxhash::FxBuildHasher>,

    kinds: Vec<RelationshipKindInfo>,
    kind_indices: HashMap<DummyId, RelationshipKindId, fxhash::FxBuildHasher>,

    // These two fields are only use for bevy to map TypeId -> DummyId, scripting/modding
    // support does not need to care about these two fields and should directly use DummyId's
    // and roll their own mapping of ScriptingId -> DummyId if necessary
    dummy_id_to_type_id: Vec<DummyInfo>,
    type_id_to_dummy_id: HashMap<TypeId, DummyId, fxhash::FxBuildHasher>,
}

impl Default for Relationships {
    fn default() -> Self {
        let mut this = Self {
            relationships: Default::default(),
            relationship_indices: Default::default(),

            kinds: Default::default(),
            kind_indices: Default::default(),

            dummy_id_to_type_id: Default::default(),
            type_id_to_dummy_id: Default::default(),
        };

        let has_component_id =
            this.new_dummy_id(Some(TypeId::of::<relationship_kinds::HasComponent>()));
        this.new_relationship_kind(has_component_id);

        let has_resource_id =
            this.new_dummy_id(Some(TypeId::of::<relationship_kinds::HasResource>()));
        this.new_relationship_kind(has_resource_id);

        this
    }
}

#[derive(Debug, Error)]
pub enum RelationshipsError {
    #[error("A relationship of type {0:?} already exists")]
    RelationshipAlreadyExists(Relship),
}

impl Relationships {
    pub fn relkind_of_has_component(&self) -> RelationshipKindId {
        let has_component_id =
            self.type_id_to_dummy_id[&TypeId::of::<relationship_kinds::HasComponent>()];
        self.kind_indices[&has_component_id]
    }

    pub fn relkind_of_has_resource(&self) -> RelationshipKindId {
        let has_resource_id =
            self.type_id_to_dummy_id[&TypeId::of::<relationship_kinds::HasResource>()];
        self.kind_indices[&has_resource_id]
    }

    pub fn new_relationship_kind(&mut self, dummy_id: DummyId) -> RelationshipKindId {
        let id = RelationshipKindId(self.kinds.len());
        self.kind_indices.insert(dummy_id, id);
        self.kinds.push(RelationshipKindInfo { id });
        id
    }

    /// TypeId is used by bevy to register a mapping from typeid -> dummyid  
    /// dynamic component use of this should pass in None or else it could
    /// interfere with bevy's use of this `Relationships` struct
    pub(crate) fn new_dummy_id(&mut self, type_id: Option<TypeId>) -> DummyId {
        let dummy_id = DummyId(self.dummy_id_to_type_id.len());
        self.dummy_id_to_type_id.push(DummyInfo {
            rust_type: type_id,
            id: dummy_id,
        });

        if let Some(type_id) = type_id {
            let previously_inserted = self.type_id_to_dummy_id.insert(type_id, dummy_id);
            assert!(previously_inserted.is_none());
        }
        dummy_id
    }

    pub(crate) fn type_id_to_dummy_id(&self, type_id: TypeId) -> Option<DummyId> {
        self.type_id_to_dummy_id.get(&type_id).copied()
    }

    pub(crate) fn register_relationship(
        &mut self,
        relationship: Relship,
        comp_descriptor: DataLayout,
    ) -> Result<&RelationshipInfo, RelationshipsError> {
        let rel_id = RelationshipId(self.relationships.len());

        if let Entry::Occupied(_) = self.relationship_indices.entry(relationship) {
            return Err(RelationshipsError::RelationshipAlreadyExists(relationship));
        }

        self.relationship_indices.insert(relationship, rel_id);
        self.relationships.push(RelationshipInfo {
            id: rel_id,
            relationship,
            data: comp_descriptor,
        });

        // Safety: Just inserted ^^^
        unsafe { Ok(self.get_relationship_info_unchecked(rel_id)) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.relationships.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.relationships.len() == 0
    }

    #[inline]
    pub fn get_resource_id(&self, type_id: TypeId) -> Option<RelationshipId> {
        self.get_relationship_id(Relship {
            kind: self.relkind_of_has_resource(),
            target: EntityOrDummyId::DummyId(self.type_id_to_dummy_id(type_id)?),
        })
    }
    #[inline]
    pub fn get_resource_info_or_insert<T: Component>(&mut self) -> &RelationshipInfo {
        self.get_resource_info_or_insert_with(TypeId::of::<T>(), TypeInfo::of::<T>)
    }
    #[inline]
    pub fn get_non_send_resource_info_or_insert<T: Any>(&mut self) -> &RelationshipInfo {
        self.get_resource_info_or_insert_with(
            TypeId::of::<T>(),
            TypeInfo::of_non_send_and_sync::<T>,
        )
    }
    #[inline]
    fn get_resource_info_or_insert_with(
        &mut self,
        type_id: TypeId,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &RelationshipInfo {
        let component_id = match self.type_id_to_dummy_id(type_id) {
            Some(id) => id,
            None => self.new_dummy_id(Some(type_id)),
        };

        self.get_relationship_info_or_insert_with(
            Relship {
                kind: self.relkind_of_has_resource(),
                target: EntityOrDummyId::DummyId(component_id),
            },
            data_layout,
        )
    }

    #[inline]
    pub fn get_component_id(&self, type_id: TypeId) -> Option<RelationshipId> {
        self.get_relationship_id(Relship {
            kind: self.relkind_of_has_component(),
            target: EntityOrDummyId::DummyId(self.type_id_to_dummy_id(type_id)?),
        })
    }
    #[inline]
    pub fn get_component_info_or_insert<T: Component>(&mut self) -> &RelationshipInfo {
        self.get_component_info_or_insert_with(TypeId::of::<T>(), TypeInfo::of::<T>)
    }
    #[inline]
    pub(crate) fn get_component_info_or_insert_with(
        &mut self,
        type_id: TypeId,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &RelationshipInfo {
        let component_id = match self.type_id_to_dummy_id(type_id) {
            Some(id) => id,
            None => self.new_dummy_id(Some(type_id)),
        };

        self.get_relationship_info_or_insert_with(
            Relship {
                kind: self.relkind_of_has_component(),
                target: EntityOrDummyId::DummyId(component_id),
            },
            data_layout,
        )
    }

    #[inline]
    pub fn get_relationship_id(&self, relationship: Relship) -> Option<RelationshipId> {
        self.relationship_indices.get(&relationship).copied()
    }
    #[inline]
    pub fn get_relationship_info(&self, id: RelationshipId) -> Option<&RelationshipInfo> {
        self.relationships.get(id.0)
    }
    /// # Safety
    /// `id` must be a valid [RelationshipId]
    #[inline]
    pub unsafe fn get_relationship_info_unchecked(&self, id: RelationshipId) -> &RelationshipInfo {
        debug_assert!(id.index() < self.relationships.len());
        self.relationships.get_unchecked(id.0)
    }
    #[inline]
    pub fn get_relationship_info_or_insert_with(
        &mut self,
        relationship: Relship,
        data_layout: impl FnOnce() -> TypeInfo,
    ) -> &RelationshipInfo {
        let Relationships {
            relationship_indices,
            relationships,
            ..
        } = self;

        let id = *relationship_indices.entry(relationship).or_insert_with(|| {
            let rel_id = RelationshipId(relationships.len());

            relationships.push(RelationshipInfo {
                id: rel_id,
                relationship,
                data: data_layout().into(),
            });

            rel_id
        });

        // Safety: just inserted
        unsafe { self.get_relationship_info_unchecked(id) }
    }
}

bitflags! {
    pub struct ComponentFlags: u8 {
        const ADDED = 1;
        const MUTATED = 2;
    }
}
