// A small ledger of accounts: the Java sample tools/treesit-fuzz edits.
package sample.ledger;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.function.Predicate;

public final class Ledger {
    public enum Kind { DEBIT, CREDIT }

    public record Entry(String account, Kind kind, long cents, String memo) {
        public Entry {
            if (cents < 0) {
                throw new IllegalArgumentException("negative amount: " + cents);
            }
        }

        long signed() {
            return kind == Kind.CREDIT ? cents : -cents;
        }
    }

    private final List<Entry> entries = new ArrayList<>();
    private final Map<String, Long> balances = new HashMap<>();

    public Ledger post(String account, Kind kind, long cents, String memo) {
        Entry entry = new Entry(account, kind, cents, memo);
        entries.add(entry);
        balances.merge(account, entry.signed(), Long::sum);
        return this;
    }

    public long balance(String account) {
        return balances.getOrDefault(account, 0L);
    }

    public Optional<Entry> largest() {
        Entry best = null;
        for (Entry entry : entries) {
            if (best == null || entry.cents() > best.cents()) {
                best = entry;
            }
        }
        return Optional.ofNullable(best);
    }

    public List<Entry> select(Predicate<? super Entry> keep) {
        List<Entry> out = new ArrayList<>();
        entries.stream().filter(keep).forEach(out::add);
        return out;
    }

    public String describe(Kind kind) {
        return switch (kind) {
            case DEBIT -> "money out";
            case CREDIT -> "money in";
        };
    }

    @Override
    public String toString() {
        StringBuilder text = new StringBuilder("Ledger[");
        int i = 0;
        for (Map.Entry<String, Long> balance : balances.entrySet()) {
            if (i++ > 0) {
                text.append(", ");
            }
            text.append(balance.getKey()).append('=').append(balance.getValue());
        }
        return text.append(']').toString();
    }

    public static void main(String[] args) {
        Ledger ledger = new Ledger()
            .post("cash", Kind.CREDIT, 12_500, "salary")
            .post("cash", Kind.DEBIT, 3_200, "rent")
            .post("savings", Kind.CREDIT, 2_000, "transfer");
        System.out.println(ledger + " largest: " + ledger.largest().map(Entry::memo).orElse("none"));
    }
}
