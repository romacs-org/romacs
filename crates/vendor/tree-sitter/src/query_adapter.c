/* Copied tree and grammar facts for the Rust query engine. */
#include "language.h"
#include "tree_cursor.h"

enum {
  RomacsQueryVisible = 1,
  RomacsQueryNamed = 2,
  RomacsQuerySupertype = 4,
};

enum {
  RomacsQueryGoto,
  RomacsQueryShift,
  RomacsQueryExtra,
  RomacsQueryReduce,
  RomacsQueryOther,
};

typedef struct {
  uint16_t lookahead, kind, symbol_or_state, production, child_count;
} RomacsQueryAction;

static RomacsQueryAction romacs_query_action(TSSymbol symbol, const TSParseAction *action) {
  RomacsQueryAction result = {.lookahead = symbol, .kind = RomacsQueryOther};
  switch (action->type) {
    case TSParseActionTypeShift:
      result.kind = action->shift.extra ? RomacsQueryExtra : RomacsQueryShift;
      result.symbol_or_state = action->shift.state;
      break;
    case TSParseActionTypeReduce:
      result.kind = RomacsQueryReduce;
      result.symbol_or_state = action->reduce.symbol;
      result.production = action->reduce.production_id;
      result.child_count = action->reduce.child_count;
      break;
    default:
      break;
  }
  return result;
}

uint32_t romacs_query_actions(
  const TSLanguage *language, TSStateId state, RomacsQueryAction *out, uint32_t capacity
) {
  uint32_t count = 0;
  LookaheadIterator row = ts_language_lookaheads(language, state);
  while (ts_lookahead_iterator__next(&row)) {
    if (row.action_count) {
      for (uint32_t i = 0; i < row.action_count; i++) {
        if (count < capacity) out[count] = romacs_query_action(row.symbol, &row.actions[i]);
        count++;
      }
    } else if (row.next_state) {
      if (count < capacity) {
        out[count] = (RomacsQueryAction) {
          .lookahead = row.symbol, .kind = RomacsQueryGoto, .symbol_or_state = row.next_state,
        };
      }
      count++;
    }
  }
  return count;
}

bool romacs_query_primary_state(const TSLanguage *language, TSStateId state) {
  return ts_language_state_is_primary(language, state);
}

uint32_t romacs_query_nonterminal_start(const TSLanguage *language) {
  return language->token_count;
}

uint32_t romacs_query_nonterminal_end(const TSLanguage *language) {
  return language->symbol_count;
}

uint16_t romacs_query_symbol_flags(const TSLanguage *language, TSSymbol symbol) {
  TSSymbolMetadata metadata = ts_language_symbol_metadata(language, symbol);
  return (metadata.visible ? RomacsQueryVisible : 0)
       | (metadata.named ? RomacsQueryNamed : 0)
       | (metadata.supertype ? RomacsQuerySupertype : 0);
}

uint32_t romacs_query_symbol_aliases(
  const TSLanguage *language, TSSymbol symbol, TSSymbol *out, uint32_t capacity
) {
  const TSSymbol *start, *end;
  ts_language_aliases_for_symbol(language, symbol, &start, &end);
  uint32_t count = (uint32_t)(end - start);
  for (uint32_t i = 0; i < count && i < capacity; ++i) out[i] = start[i];
  return count;
}

void romacs_query_production_child(
  const TSLanguage *language, uint16_t production, uint16_t child, TSSymbol *alias, TSFieldId *field
) {
  *alias = 0;
  *field = 0;
  if (production >= language->production_id_count) return;
  if (child < language->max_alias_sequence_length) {
    *alias = ts_language_alias_at(language, production, child);
  }
  const TSFieldMapEntry *start, *end;
  ts_language_field_map(language, production, &start, &end);
  for (; start != end; ++start) {
    if (start->child_index == child && !start->inherited) {
      *field = start->field_id;
      break;
    }
  }
}

typedef struct {
  uint16_t field, flags, supertypes[8];
} RomacsQueryStatus;

RomacsQueryStatus romacs_query_cursor_status(const TSTreeCursor *cursor) {
  RomacsQueryStatus status = {0};
  unsigned count = 8;
  bool later, later_named, later_field;
  ts_tree_cursor_current_status(
    cursor, &status.field, &later, &later_named, &later_field, status.supertypes, &count
  );
  status.flags = (later ? 1 : 0) | (later_named ? 2 : 0) | (later_field ? 4 : 0);
  return status;
}

TSSymbol romacs_query_cursor_repetition(const TSTreeCursor *cursor) {
  Subtree subtree = ts_tree_cursor_current_subtree(cursor);
  return ts_subtree_is_repetition(subtree) ? ts_subtree_symbol(subtree) : 0;
}

typedef struct {
  uint32_t start, end;
} RomacsQuerySpan;

RomacsQuerySpan romacs_query_node_span(TSNode node) {
  return (RomacsQuerySpan) {
    .start = ts_node_start_byte(node),
    .end = ts_node_end_byte(node),
  };
}
