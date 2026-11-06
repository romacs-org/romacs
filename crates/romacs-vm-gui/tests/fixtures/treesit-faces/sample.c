/* A ring buffer of fixed-size records, as a C sample for the face oracle.  */

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define RING_CAPACITY 64
#define RING_MASK (RING_CAPACITY - 1)
#define RING_EMPTY(r) ((r)->head == (r)->tail)

enum record_kind { RECORD_TEXT = 1, RECORD_NUMBER, RECORD_FLAG = 0x10 };

typedef struct record
{
  enum record_kind kind;
  union { const char *text; double number; unsigned flag : 1; } value;
} record;

struct ring
{
  record slots[RING_CAPACITY];
  size_t head, tail;
  uint32_t dropped;
};

static inline size_t
ring_next (size_t index)
{
  return (index + 1) & RING_MASK;
}

/* Push R; when full, drop the oldest record and count it.  */
int
ring_push (struct ring *ring, const record *r)
{
  size_t next = ring_next (ring->tail);
  if (next == ring->head)
    {
      ring->head = ring_next (ring->head);
      ring->dropped++;
    }
  memcpy (&ring->slots[ring->tail], r, sizeof *r);
  ring->tail = next;
  return ring->dropped > 0 ? -1 : 0;
}

double
ring_sum (const struct ring *ring)
{
  double sum = 0.0;
  for (size_t i = ring->head; i != ring->tail; i = ring_next (i))
    switch (ring->slots[i].kind)
      {
      case RECORD_NUMBER:
	sum += ring->slots[i].value.number;
	break;
      case RECORD_TEXT:
	sum += strlen (ring->slots[i].value.text) * 1e-3;
	break;
      default:
	continue;
      }
  return sum;
}

const char *
ring_describe (const struct ring *ring)
{
  static char line[32];
  char sep = ',';
  if (RING_EMPTY (ring))
    return "empty";
  line[0] = sep == ',' ? 'r' : '\0';
  return line;
}
