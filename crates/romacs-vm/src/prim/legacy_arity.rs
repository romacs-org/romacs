use crate::Primitive;

pub(super) fn arity(primitive: Primitive) -> (usize, Option<usize>) {
    use Primitive::*;
    match primitive {
        Add | Sub | Mul | Vector | Insert | ByteString | List | Nconc | InsertInherit
        | InsertBeforeMarkers | MakeHashTable | Append => (0, None),
        Div | Less | Equal | ProcessLines | Format | Message | Max | DisplayError | Min => {
            (1, None)
        }
        Abs
        | Inc
        | Dec
        | Car
        | Cdr
        | Length
        | Nreverse
        | CopySequence
        | SetBuffer
        | GotoChar
        | CharToByte
        | ByteToChar
        | SetMultibyte
        | EncodeUtf8
        | DecodeUtf8
        | StringToBytes
        | IsMultibyte
        | InsertFileBytes
        | Stringp
        | Null
        | FileDirectoryP
        | Boundp
        | BareSymbol
        | YankPart
        | MarkerPosition
        | MarkerBuffer
        | MarkerInsertionType
        | DeleteOverlay
        | OverlayStart
        | OverlayEnd
        | OverlaysAt
        | NextOverlayChange
        | Consp
        | CarSafe
        | Symbolp
        | ErrorMessage
        | Clrhash
        | HashTableCount
        | HashTableP
        | HashTableTest
        | HashTableSize
        | HashTableWeakness
        | HashTableRehashSize
        | HashTableRehashThreshold
        | CopyHashTable
        | HashTableKeys
        | HashTableValues
        | Integerp
        | Fixnump
        | Bignump
        | MatchBeginning
        | MatchEnd
        | SetSyntaxTable
        | DeleteWindowInternal
        | Windowp
        | WindowLiveP
        | Keymapp
        | UseLocalMap
        | Kbd
        | SymbolValue
        | SpecialVariableP
        | MakeVariableBufferLocal
        | LocalVariableIfSetP
        | CustomVariableP
        | DefvarBoundp
        | GetBuffer
        | Fboundp
        | Makunbound
        | MakeLocalVariable => (1, Some(1)),
        Cons
        | Throw
        | Aref
        | Assq
        | PlistGet
        | Setcar
        | Setcdr
        | Remainder
        | DeleteRegion
        | Substring
        | Narrow
        | Member
        | Nth
        | Get
        | Eq
        | Memq
        | BufferSubstring
        | SetMarkerInsertionType
        | OverlayGet
        | OverlaysIn
        | Remhash
        | Maphash
        | HashTableContainsP
        | HashNext
        | HashKeyAt
        | HashValueAt
        | DisplayDefineFace
        | Fset
        | SetWindowPoint
        | WindowParameter
        | GlobalSetKey
        | DefvarSetDefault
        | DeepEqual
        | Set => (2, Some(2)),
        Apply => (1, None),
        Aset
        | Put
        | YankPropertyRun
        | YankFontLockFace
        | YankCategory
        | OverlayPut
        | Puthash
        | DefineHashTableTest
        | SetWindowParameter
        | MakeObsolete
        | SetAdvertisedCallingConvention => (3, Some(3)),
        CurrentBuffer | Point | PointMin | PointMax | BufferSize | Widen
        | BufferString | EncodeBufferUtf8 | DecodeBufferUtf8 | CacheStats | CacheClear
        | CacheEntries | MakeMarker | SyntaxTable | StandardSyntaxTable | SelectedWindow
        | SelectedFrame | CurrentGlobalMap | EmacsLispMode | Daemonp
        | DaemonInitialized => (0, Some(0)),
        ReadHostEvent => (0, Some(2)),
        FormatModeLine => (1, Some(4)),
        CharAfter | ForwardChar | BackwardChar | BeginningOfLine | EndOfLine | CopySyntaxTable
        | WindowBuffer | WindowStart | WindowPoint | FramePixelWidth | FramePixelHeight
        | FrameSelectedWindow | MinibufferWindow | WindowNewPixel | WindowParent
        | WindowNextSibling | WindowPrevSibling | WindowChild | FrameRootWindow
        | WindowParameters | WindowMargins | WindowPixelWidth | WindowPixelHeight
        | MakeSparseKeymap | ForwardLine | MenuBarMode | ForceModeLineUpdate | TempBuffer => {
            (0, Some(1))
        }
        Eval | Provide | Featurep
        | ExpandFileName | RawPropertiesAt | KillNew | CurrentKill | CopyMarker | LookingAt
        | MatchString | Macroexpand | Signal => (1, Some(2)),
        Require | Prin1ToString | SwitchToBuffer | AutoloadDoLoad => (1, Some(3)),
        SplitWindowInternal => (4, Some(5)),
        GetProperty | Defalias | GetCharProperty | SetMarker | DefineError | Gethash
        | ModifySyntaxEntry | SetWindowMargins | SetWindowStart
        | SetWindowNewPixel | LookupKey | ClosureDocument => (2, Some(3)),
        Load => (1, Some(5)),
        YankFinalize => (4, Some(4)),
        MakeOverlay => (2, Some(5)),
        MoveOverlay | DefineKey | MakeObsoleteVariable => (3, Some(4)),
        SetWindowVscroll => (2, Some(4)),
        RemoveOverlays => (0, Some(4)),
        StringMatch => (2, Some(4)),
        MatchData => (0, Some(3)),
        ReSearchForward | ReSearchBackward | SearchForward | SearchBackward | KeyBinding | LocateLibrary => (1, Some(4)),
        WindowVscroll | WindowTotalHeight | WindowResizeApply | WindowTotalWidth => {
            (0, Some(2))
        }
        ClosureMetadata => (6, Some(6)),
        MakeInterpretedClosure => (5, Some(5)),
    }
}
