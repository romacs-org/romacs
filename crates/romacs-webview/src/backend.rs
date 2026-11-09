use romacs_display_protocol::WebViewId;

use crate::{
    BrowsingRelationship, FocusIntent, HistoryAction, HostWindowId, NavigationTarget,
    ResolvedWebViewPlacement, ScriptRequest, StoragePartition, WebContentSize, WebViewEvent,
    WebViewFrame, WebViewGeneration, WebViewInput, WebViewPolicy,
};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct MissingPrerequisites: u8 {
        const PROFILE = 1 << 0;
        const HOST = 1 << 1;
        const GPU = 1 << 2;
        const RELATED_VIEW = 1 << 3;
        const RUNTIME = 1 << 4;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlatformCreateRequest {
    id: WebViewId,
    generation: WebViewGeneration,
    storage: StoragePartition,
    relationship: BrowsingRelationship,
    size: WebContentSize,
    policy: WebViewPolicy,
    navigation: Option<NavigationTarget>,
}

impl PlatformCreateRequest {
    pub(crate) fn new(
        id: WebViewId,
        generation: WebViewGeneration,
        storage: StoragePartition,
        relationship: BrowsingRelationship,
        size: WebContentSize,
        policy: WebViewPolicy,
        navigation: Option<NavigationTarget>,
    ) -> Self {
        Self {
            id,
            generation,
            storage,
            relationship,
            size,
            policy,
            navigation,
        }
    }

    pub(crate) const fn id(&self) -> WebViewId {
        self.id
    }

    pub(crate) const fn generation(&self) -> WebViewGeneration {
        self.generation
    }

    pub(crate) const fn size(&self) -> WebContentSize {
        self.size
    }

    pub(crate) const fn navigation(&self) -> Option<&NavigationTarget> {
        self.navigation.as_ref()
    }

    pub(crate) const fn storage(&self) -> &StoragePartition {
        &self.storage
    }

    pub(crate) const fn relationship(&self) -> &BrowsingRelationship {
        &self.relationship
    }

    pub(crate) const fn policy(&self) -> &WebViewPolicy {
        &self.policy
    }
}

// The WPE adapter creates asynchronously; the test platform also creates inline.
#[allow(dead_code)]
pub(crate) enum CreateOutcome<V, C> {
    Ready(V),
    Pending(C),
}

#[derive(Debug)]
// Kept in the platform contract because native engines may complete creation
// asynchronously; the deterministic fake platform exercises stale completion
// handling even when the current native adapters complete inline.
#[allow(dead_code)]
pub(crate) enum BackendEvent<V> {
    CreateFinished {
        id: WebViewId,
        generation: WebViewGeneration,
        result: Result<V, String>,
    },
}

pub(crate) enum PlatformUpdate<'a> {
    ModelSize(WebContentSize),
    Navigation(&'a NavigationTarget),
    History(HistoryAction),
    EvaluateScript(&'a ScriptRequest),
    Focus(FocusIntent),
}

// Native host placement, for an adapter that overlays a native view. The WPE
// adapter composites, so it ignores it.
pub(crate) enum PlatformPresentation<'a> {
    Hidden,
    Visible {
        host: HostWindowId,
        placement: &'a ResolvedWebViewPlacement,
    },
}

/// What registering a native host changed at the platform seam.
///
/// A logical [`HostWindowId`] can survive recreation of the native window
/// behind it.  The system must reapply its current scene after
/// [`HostRegistration::Replaced`], even when redisplay produced no new scene
/// revision; otherwise native-overlay views remain attached to the dead host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
// A native-overlay adapter would use every outcome; the WPE and unsupported
// adapters construct only a subset.
pub(crate) enum HostRegistration {
    Added,
    Unchanged,
    Replaced,
    Unavailable,
}

pub(crate) trait Platform {
    type Host;
    type PendingCreate;
    type View;

    fn register_host(&mut self, id: HostWindowId, host: Self::Host) -> HostRegistration;
    fn unregister_host(&mut self, host: HostWindowId);

    fn missing_prerequisites(&self, request: &PlatformCreateRequest) -> MissingPrerequisites;

    fn begin_create(
        &mut self,
        request: PlatformCreateRequest,
    ) -> Result<CreateOutcome<Self::View, Self::PendingCreate>, String>;

    fn drain_events(&mut self) -> Vec<BackendEvent<Self::View>>;

    /// Give a platform-owned pending creation the presentation capability it
    /// needs to finish: a native-overlay adapter could not create its view
    /// until a concrete host window is known. The WPE adapter needs none.
    fn activate_pending(
        &mut self,
        _generation: WebViewGeneration,
        _pending: &mut Self::PendingCreate,
        _presentation: PlatformPresentation<'_>,
    ) -> Result<Option<Self::View>, String> {
        Ok(None)
    }

    fn service_view(
        &mut self,
        _id: WebViewId,
        _generation: WebViewGeneration,
        _view: &mut Self::View,
    ) -> Vec<WebViewEvent> {
        Vec::new()
    }

    fn take_frame(&mut self, _view: &mut Self::View) -> Option<WebViewFrame> {
        None
    }

    fn has_pending_frame(&self, _view: &Self::View) -> bool {
        false
    }

    /// Native-overlay backends do not export DMA-BUFs. Composited backends
    /// replace their receiving-device capabilities without recreating pages.
    fn set_dma_buf_import_formats(
        &mut self,
        _formats: romacs_display_protocol::DmaBufImportFormats,
    ) -> Result<(), String> {
        Ok(())
    }

    fn update(
        &mut self,
        _view: &mut Self::View,
        _update: PlatformUpdate<'_>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn input(
        &mut self,
        _generation: WebViewGeneration,
        _view: &mut Self::View,
        _input: WebViewInput,
    ) -> Result<(), String> {
        Ok(())
    }

    fn present(
        &mut self,
        _generation: WebViewGeneration,
        _view: &mut Self::View,
        _presentation: PlatformPresentation<'_>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn close(&mut self, _view: Self::View) {}
}
