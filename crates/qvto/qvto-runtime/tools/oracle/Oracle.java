// Java oracle for qvto-runtime's differential tests. Reads one request per line from stdin and
// writes one result line per request. Requires Eclipse OCL's jar (CollectionUtil, BagImpl) and
// QVTo's (DictionaryImpl) on the class path: see run.sh.
//
// Value encoding (requests and results):
//   n null | s<hex UTF-16 units, 4 digits each> | i<int> | l<long> | d<16 hex bits> |
//   f<8 hex bits> | b1 b0 | h<short> | y<byte> | c<4 hex> | o<id>.<hash> (object with that
//   identity hash; results print o<id>) | r<reg> (register value, requests only) |
//   collections in results: S[..] HashSet, O[..] LinkedHashSet, B[..] BagImpl (iteration order,
//   repeats), Q[..] ArrayList / any List, D[k=v,..] DictionaryImpl, X invalid.
//
// Requests:
//   hash <v>                     Objects.hashCode
//   coll <stmt>;<stmt>;...       collection script (see exec), prints the p statements' output
//   dtoa <hex bits> / ftoa <hex bits>
//   dparse <hex utf16> / iparse <hex utf16> / lparse <hex utf16>
//   str <op> <hex utf16 args...> String operations (see str())
//   char <code unit>             Character data of a UTF-16 unit
//   cp <code point>              case mappings of a code point
//   ocl <hex utf16 OCL expression>  classic OCL evaluation (long integers on), see ocl()
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

import org.eclipse.ocl.util.Bag;
import org.eclipse.ocl.util.CollectionUtil;

public class Oracle {
    static final class IdObj {
        final int id, h;
        IdObj(int id, int h) { this.id = id; this.h = h; }
        @Override public int hashCode() { return h; }
    }

    static final Object INVALID = new Object() { public String toString() { return "invalid"; } };
    static final Map<Integer, IdObj> objs = new HashMap<>();
    static final Object[] regs = new Object[32];

    public static void main(String[] args) throws Exception {
        Locale.setDefault(Locale.US);
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintStream out = new PrintStream(new BufferedOutputStream(System.out, 1 << 16), false, "UTF-8");
        String line;
        while ((line = in.readLine()) != null) {
            String res;
            try {
                res = run(line);
            } catch (Throwable t) {
                res = "EXC " + t.getClass().getSimpleName();
            }
            out.println(res);
        }
        out.flush();
    }

    static String run(String line) throws Exception {
        objs.clear();
        int sp = line.indexOf(' ');
        String cmd = sp < 0 ? line : line.substring(0, sp);
        String arg = sp < 0 ? "" : line.substring(sp + 1);
        switch (cmd) {
            case "hash": return Integer.toString(Objects.hashCode(dec(arg)));
            case "coll": return coll(arg);
            case "dtoa": return Double.toString(Double.longBitsToDouble(Long.parseUnsignedLong(arg, 16)));
            case "ftoa": return Float.toString(Float.intBitsToFloat(Integer.parseUnsignedInt(arg, 16)));
            case "dparse":
                try { return Long.toHexString(Double.doubleToRawLongBits(Double.valueOf(unhex(arg)))); }
                catch (NumberFormatException e) { return "NFE"; }
            case "iparse":
                try { return Integer.valueOf(unhex(arg)).toString(); }
                catch (NumberFormatException e) { return "NFE"; }
            case "lparse":
                try { return Long.toString(Long.parseLong(unhex(arg))); }
                catch (NumberFormatException e) { return "NFE"; }
            case "str": return str(arg.split(" ", -1));
            case "char": return charData(Integer.parseInt(arg));
            case "cp": return cpData(Integer.parseInt(arg));
            case "ocl": return OclEval.eval(unhex(arg));
            default: throw new IllegalArgumentException(cmd);
        }
    }

    // ---------------------------------------------------------------- encoding

    static String hex(String s) {
        StringBuilder b = new StringBuilder(s.length() * 4);
        for (int i = 0; i < s.length(); i++) b.append(String.format("%04x", (int) s.charAt(i)));
        return b.toString();
    }

    static String unhex(String h) {
        StringBuilder b = new StringBuilder(h.length() / 4);
        for (int i = 0; i + 4 <= h.length(); i += 4) b.append((char) Integer.parseInt(h.substring(i, i + 4), 16));
        return b.toString();
    }

    static Object dec(String t) {
        char k = t.charAt(0);
        String r = t.substring(1);
        switch (k) {
            case 'n': return null;
            case 's': return unhex(r);
            case 'i': return Integer.valueOf(r);
            case 'l': return Long.valueOf(r);
            case 'd': return Double.longBitsToDouble(Long.parseUnsignedLong(r, 16));
            case 'f': return Float.intBitsToFloat(Integer.parseUnsignedInt(r, 16));
            case 'b': return r.equals("1");
            case 'h': return Short.valueOf(r);
            case 'y': return Byte.valueOf(r);
            case 'c': return (char) Integer.parseInt(r, 16);
            case 'o': {
                int dot = r.indexOf('.');
                int id = Integer.parseInt(r.substring(0, dot));
                int h = Integer.parseInt(r.substring(dot + 1));
                return objs.computeIfAbsent(id, i -> new IdObj(i, h));
            }
            case 'r': return regs[Integer.parseInt(r)];
            default: throw new IllegalArgumentException(t);
        }
    }

    static String enc(Object o) {
        if (o == null) return "n";
        if (o == INVALID) return "X";
        if (o instanceof String) return "s" + hex((String) o);
        if (o instanceof Integer) return "i" + o;
        if (o instanceof Long) return "l" + o;
        if (o instanceof Double) return "d" + String.format("%016x", Double.doubleToRawLongBits((Double) o));
        if (o instanceof Float) return "f" + String.format("%08x", Float.floatToRawIntBits((Float) o));
        if (o instanceof Boolean) return ((Boolean) o) ? "b1" : "b0";
        if (o instanceof Short) return "h" + o;
        if (o instanceof Byte) return "y" + o;
        if (o instanceof Character) return "c" + String.format("%04x", (int) (Character) o);
        if (o instanceof IdObj) return "o" + ((IdObj) o).id;
        if (o instanceof org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl) {
            // keys() and values() iterate the same HashMap
            org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<?, ?> d =
                (org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<?, ?>) o;
            List<?> ks = d.keys(), vs = d.values();
            StringBuilder b = new StringBuilder("D[");
            for (int i = 0; i < ks.size(); i++) {
                if (i > 0) b.append(',');
                b.append(enc(ks.get(i))).append('=').append(enc(vs.get(i)));
            }
            return b.append(']').toString();
        }
        if (o instanceof Collection) {
            String k;
            if (o instanceof LinkedHashSet) k = "O";
            else if (o instanceof Set) k = "S";
            else if (o instanceof Bag) k = "B";
            else if (o instanceof List) k = "Q";
            else k = "?" + o.getClass().getName();
            StringBuilder b = new StringBuilder(k).append('[');
            boolean first = true;
            for (Object e : (Collection<?>) o) {
                if (!first) b.append(',');
                first = false;
                b.append(enc(e));
            }
            return b.append(']').toString();
        }
        return "?" + o.getClass().getName();
    }

    // ---------------------------------------------------------------- collection scripts

    static void emit(StringBuilder out, String s) {
        if (out.length() > 0) out.append(' ');
        out.append(s);
    }

    static int reg(String s) { return Integer.parseInt(s); }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static Collection<Object> c(String s) { return (Collection<Object>) regs[reg(s)]; }

    static CollectionKindLike kind(String k) { return CollectionKindLike.valueOf(k); }

    enum CollectionKindLike { S, O, B, Q }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static Collection<Object> newOf(String k) {
        switch (k) {
            case "S": return CollectionUtil.createNewSet();
            case "O": return CollectionUtil.createNewOrderedSet();
            case "B": return CollectionUtil.createNewBag();
            case "Q": return CollectionUtil.createNewSequence();
            default: throw new IllegalArgumentException(k);
        }
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static Collection<Object> copyOf(String k, Collection<Object> src) {
        switch (k) {
            case "S": return CollectionUtil.createNewSet(src);
            case "O": return CollectionUtil.createNewOrderedSet(src);
            case "B": return CollectionUtil.createNewBag(src);
            case "Q": return CollectionUtil.createNewSequence(src);
            default: throw new IllegalArgumentException(k);
        }
    }

    /** QVTo's Set::including/excluding handlers: a LinkedHashSet source becomes a HashSet first. */
    static Collection<Object> qvtoSrc(Collection<Object> s) {
        return s instanceof LinkedHashSet ? CollectionUtil.createNewSet(s) : s;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String coll(String script) {
        Arrays.fill(regs, null);
        StringBuilder out = new StringBuilder();
        for (String stmt : script.split(";")) {
            String[] t = stmt.trim().split(" ");
            if (t.length == 0 || t[0].isEmpty()) continue;
            Object res = null;
            boolean assign = true;
            int dst = -1;
            try {
                switch (t[0]) {
                    case "new": dst = reg(t[2]); res = newOf(t[1]); break;
                    case "dict": dst = reg(t[1]); res = new org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<Object, Object>(); break;
                    case "+": assign = false; c(t[1]).add(dec(t[2])); break;
                    case "-": assign = false; c(t[1]).remove(dec(t[2])); break;
                    case "clr": assign = false; c(t[1]).clear(); break;
                    case "ret": { // iterator removal of every k-th element (1-based)
                        assign = false;
                        int k = Integer.parseInt(t[2]), i = 0;
                        for (Iterator<Object> it = c(t[1]).iterator(); it.hasNext(); ) {
                            it.next();
                            if (++i % k == 0) it.remove();
                        }
                        break;
                    }
                    case "rall": assign = false; c(t[1]).removeAll(c(t[2])); break;
                    case "addall": assign = false; c(t[1]).addAll(c(t[2])); break;
                    case "cp": dst = reg(t[2]); res = copyOf(t[1], c(t[3])); break;
                    case "inc": dst = reg(t[1]); res = CollectionUtil.including(c(t[2]), dec(t[3])); break;
                    case "exc": dst = reg(t[1]); res = CollectionUtil.excluding(c(t[2]), dec(t[3])); break;
                    case "qinc": dst = reg(t[1]); res = CollectionUtil.including(qvtoSrc(c(t[2])), dec(t[3])); break;
                    case "qexc": dst = reg(t[1]); res = CollectionUtil.excluding(qvtoSrc(c(t[2])), dec(t[3])); break;
                    case "uni": dst = reg(t[1]); res = CollectionUtil.union(c(t[2]), c(t[3])); break;
                    case "int": dst = reg(t[1]); res = CollectionUtil.intersection(c(t[2]), c(t[3])); break;
                    case "min": dst = reg(t[1]); res = CollectionUtil.minus((Set) c(t[2]), (Set) c(t[3])); break;
                    case "sym": dst = reg(t[1]); res = CollectionUtil.symmetricDifference((Set) c(t[2]), (Set) c(t[3])); break;
                    case "fla": dst = reg(t[1]); res = CollectionUtil.flatten(c(t[2])); break;
                    case "qfla": dst = reg(t[1]); res = CollectionUtil.flatten(qvtoSrc(c(t[2]))); break;
                    case "aset": dst = reg(t[1]); res = CollectionUtil.asSet(c(t[2])); break;
                    case "abag": dst = reg(t[1]); res = CollectionUtil.asBag(c(t[2])); break;
                    case "aseq": dst = reg(t[1]); res = CollectionUtil.asSequence(c(t[2])); break;
                    case "aos": dst = reg(t[1]); res = CollectionUtil.asOrderedSet(c(t[2])); break;
                    case "app": dst = reg(t[1]); res = CollectionUtil.append(c(t[2]), dec(t[3])); break;
                    case "pre": dst = reg(t[1]); res = CollectionUtil.prepend(c(t[2]), dec(t[3])); break;
                    case "ins": dst = reg(t[1]); res = CollectionUtil.insertAt(c(t[2]), Integer.parseInt(t[3]), dec(t[4])); break;
                    case "subs": dst = reg(t[1]); res = CollectionUtil.subSequence(c(t[2]), Integer.parseInt(t[3]), Integer.parseInt(t[4])); break;
                    case "subo": dst = reg(t[1]); res = CollectionUtil.subOrderedSet(c(t[2]), Integer.parseInt(t[3]), Integer.parseInt(t[4])); break;
                    case "dput": assign = false; ((org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<Object, Object>) regs[reg(t[1])]).put(dec(t[2]), dec(t[3])); break;
                    case "dcp": dst = reg(t[1]); res = new org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<Object, Object>((org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<Object, Object>) regs[reg(t[2])]); break;
                    case "drem": assign = false; ((org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl<Object, Object>) regs[reg(t[1])]).remove(dec(t[2])); break;
                    case "p": assign = false; emit(out, String.valueOf(enc(regs[reg(t[1])]))); break;
                    case "ph": assign = false; emit(out, "#" + Objects.hashCode(regs[reg(t[1])])); break;
                    case "pin": assign = false; emit(out, String.valueOf(CollectionUtil.includes(c(t[1]), dec(t[2])) ? "T" : "F")); break;
                    case "pat": assign = false; emit(out, String.valueOf(enc(CollectionUtil.at(c(t[1]), Integer.parseInt(t[2]))))); break;
                    case "pidx": assign = false; { Integer ix = CollectionUtil.indexOf(c(t[1]), dec(t[2])); emit(out, String.valueOf(ix == null ? "X" : "i" + ix)); } break;
                    case "pcnt": assign = false; emit(out, "i" + CollectionUtil.count(c(t[1]), dec(t[2]))); break;
                    case "peq": assign = false; emit(out, String.valueOf(org.eclipse.ocl.util.ObjectUtil.equal(regs[reg(t[1])], regs[reg(t[2])]) ? "T" : "F")); break;
                    case "pjeq": assign = false; emit(out, String.valueOf(Objects.equals(regs[reg(t[1])], regs[reg(t[2])]) ? "T" : "F")); break;
                    case "psum": assign = false; { Object s = CollectionUtil.sum(c(t[1])); emit(out, String.valueOf(enc(s))); } break;
                    case "pmax": assign = false; emit(out, String.valueOf(enc(CollectionUtil.max(c(t[1]))))); break;
                    case "pmin": assign = false; emit(out, String.valueOf(enc(CollectionUtil.min(c(t[1]))))); break;
                    default: throw new IllegalArgumentException(t[0]);
                }
            } catch (IllegalArgumentException e) {
                if (e.getClass() == IllegalArgumentException.class && e.getMessage() != null && e.getMessage().equals(t[0])) throw e;
                res = INVALID;
                if (!assign) emit(out, "X");
            } catch (RuntimeException e) {
                res = INVALID;
                if (!assign) emit(out, "X");
            }
            if (assign && dst >= 0) regs[dst] = res;
        }
        return out.toString();
    }

    // ---------------------------------------------------------------- strings and characters

    static String str(String[] a) {
        String op = a[0];
        String s = unhex(a[1]);
        switch (op) {
            case "hash": return Integer.toString(s.hashCode());
            case "len": return Integer.toString(s.length());
            case "upper": return hex(s.toUpperCase());
            case "lower": return hex(s.toLowerCase());
            case "trim": return hex(s.trim());
            case "cmp": return Integer.toString(Integer.signum(s.compareTo(unhex(a[2]))));
            case "eqic": return s.equalsIgnoreCase(unhex(a[2])) ? "T" : "F";
            case "idx": return Integer.toString(s.indexOf(unhex(a[2])));
            case "lidx": return Integer.toString(s.lastIndexOf(unhex(a[2])));
            case "repl": return hex(s.replace(unhex(a[2]), unhex(a[3])));
            case "sw": return s.startsWith(unhex(a[2])) ? "T" : "F";
            case "ew": return s.endsWith(unhex(a[2])) ? "T" : "F";
            case "sub": return hex(s.substring(Integer.parseInt(a[2]), Integer.parseInt(a[3])));
            case "tok": {
                StringTokenizer st = a.length > 2 ? new StringTokenizer(s, unhex(a[2])) : new StringTokenizer(s);
                StringBuilder b = new StringBuilder();
                while (st.hasMoreTokens()) b.append(b.length() == 0 ? "" : ",").append(hex(st.nextToken()));
                return "[" + b + "]";
            }
            default: throw new IllegalArgumentException(op);
        }
    }

    static String charData(int u) {
        char c = (char) u;
        return (Character.isWhitespace(c) ? "W" : "-") + (Character.isLetter(c) ? "L" : "-")
            + (Character.isLetterOrDigit(c) ? "D" : "-") + " " + Character.digit(c, 10)
            + " " + (int) Character.toUpperCase(c) + " " + (int) Character.toLowerCase(c);
    }

    static String cpData(int cp) {
        String s = new String(Character.toChars(cp));
        return hex(s.toUpperCase(Locale.ROOT)) + " " + hex(s.toLowerCase(Locale.ROOT)) + " "
            + Character.toUpperCase(cp) + " " + Character.toLowerCase(cp) + " "
            + (Character.isLowerCase(cp) || Character.isUpperCase(cp) || Character.isTitleCase(cp) ? "C" : "-");
    }
}
