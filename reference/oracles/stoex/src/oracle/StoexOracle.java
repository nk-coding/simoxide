package oracle;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.FileReader;
import java.io.FileWriter;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.SplittableRandom;

import org.eclipse.emf.ecore.EObject;
import org.palladiosimulator.pcm.parameter.CharacterisedVariable;
import org.palladiosimulator.pcm.stoex.api.StoExParser;
import org.palladiosimulator.pcm.stoex.api.StoExSerialiser;

import de.uka.ipd.sdq.probfunction.BoolSample;
import de.uka.ipd.sdq.probfunction.BoxedPDF;
import de.uka.ipd.sdq.probfunction.ContinuousSample;
import de.uka.ipd.sdq.probfunction.DoubleSample;
import de.uka.ipd.sdq.probfunction.IntSample;
import de.uka.ipd.sdq.probfunction.ProbabilityMassFunction;
import de.uka.ipd.sdq.probfunction.Sample;
import de.uka.ipd.sdq.probfunction.StringSample;
import de.uka.ipd.sdq.probfunction.math.IProbabilityFunctionFactory;
import de.uka.ipd.sdq.probfunction.math.IRandomGenerator;
import de.uka.ipd.sdq.probfunction.math.impl.ProbabilityFunctionFactoryImpl;
import de.uka.ipd.sdq.simucomframework.variables.EvaluationProxy;
import de.uka.ipd.sdq.simucomframework.variables.StackContext;
import de.uka.ipd.sdq.simucomframework.variables.cache.StoExCache;
import de.uka.ipd.sdq.simucomframework.variables.stackframe.SimulatedStackframe;
import de.uka.ipd.sdq.simucomframework.variables.stoexvisitor.VariableMode;
import de.uka.ipd.sdq.stoex.AbstractNamedReference;
import de.uka.ipd.sdq.stoex.BoolLiteral;
import de.uka.ipd.sdq.stoex.BooleanOperatorExpression;
import de.uka.ipd.sdq.stoex.CompareExpression;
import de.uka.ipd.sdq.stoex.DoubleLiteral;
import de.uka.ipd.sdq.stoex.Expression;
import de.uka.ipd.sdq.stoex.FunctionLiteral;
import de.uka.ipd.sdq.stoex.IfElseExpression;
import de.uka.ipd.sdq.stoex.IntLiteral;
import de.uka.ipd.sdq.stoex.NamespaceReference;
import de.uka.ipd.sdq.stoex.NegativeExpression;
import de.uka.ipd.sdq.stoex.NotExpression;
import de.uka.ipd.sdq.stoex.Parenthesis;
import de.uka.ipd.sdq.stoex.PowerExpression;
import de.uka.ipd.sdq.stoex.ProbabilityFunctionLiteral;
import de.uka.ipd.sdq.stoex.ProductExpression;
import de.uka.ipd.sdq.stoex.StringLiteral;
import de.uka.ipd.sdq.stoex.TermExpression;
import de.uka.ipd.sdq.stoex.VariableReference;
import de.uka.ipd.sdq.stoex.analyser.visitors.NonProbabilisticExpressionInferTypeVisitor;
import de.uka.ipd.sdq.stoex.analyser.visitors.TypeEnum;

/**
 * StoEx golden-file oracle: parses, type-infers, prepares and evaluates StoEx cases with the real
 * SimuLizar 5.2.2 classes (StoExParser, StoExCache, PCMStoExEvaluationVisitor via
 * StackContext.evaluateStatic) and writes one JSON line per case.
 *
 * Usage: StoexOracle CASES.jsonl VARSETS.json GOLDEN.jsonl
 *
 * Case: {"id":..., "expr":..., "mode":"EXCEPTION"|"DEFAULT"|"NULL", "evals":N, "seed":S,
 * "uniforms":[...], "vars":[{"id":"x.VALUE","t":"int|double|bool|string|proxy","v":...}]}
 * Doubles may be given as "0x<16 hex digits>" (raw bits). Uniforms: the explicit list first, then
 * SplittableRandom(seed).nextDouble().
 */
public class StoexOracle {

    /** Uniform source that records every draw. */
    static final class Tape implements IRandomGenerator {
        List<Double> explicit = new ArrayList<>();
        int pos;
        SplittableRandom rnd = new SplittableRandom(0);
        List<Double> drawn = new ArrayList<>();

        void reset(List<Double> explicit, long seed) {
            this.explicit = explicit;
            this.pos = 0;
            this.rnd = new SplittableRandom(seed);
        }

        @Override
        public double random() {
            double u = pos < explicit.size() ? explicit.get(pos++) : rnd.nextDouble();
            drawn.add(u);
            return u;
        }

        @Override
        public void dispose() {
        }
    }

    static final Tape TAPE = new Tape();
    static final StoExSerialiser SER = StoExSerialiser.createInstance();

    static Map<String, Object> VARSETS;

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        IProbabilityFunctionFactory factory = ProbabilityFunctionFactoryImpl.getInstance();
        factory.setRandomGenerator(TAPE);
        StoExCache.initialiseStoExCache(factory);
        StoExParser parser = StoExParser.createInstance();
        org.apache.log4j.Logger.getRootLogger().setLevel(org.apache.log4j.Level.OFF);

        int n = 0;
        VARSETS = (Map<String, Object>) new Json(new String(java.nio.file.Files.readAllBytes(java.nio.file.Paths.get(args[1])),
                java.nio.charset.StandardCharsets.UTF_8)).value();
        try (BufferedReader r = new BufferedReader(new FileReader(args[0]));
                BufferedWriter w = new BufferedWriter(new FileWriter(args[2]))) {
            String line;
            while ((line = r.readLine()) != null) {
                if (line.isBlank()) {
                    continue;
                }
                @SuppressWarnings("unchecked")
                Map<String, Object> c = (Map<String, Object>) new Json(line).value();
                w.write(runCase(parser, c));
                w.write('\n');
                n++;
            }
        }
        System.err.println("oracle: " + n + " cases");
    }

    @SuppressWarnings("unchecked")
    static String runCase(StoExParser parser, Map<String, Object> c) {
        String expr = (String) c.get("expr");
        StringBuilder o = new StringBuilder();
        o.append("{\"id\":").append(q(String.valueOf(c.get("id"))));
        o.append(",\"expr\":").append(q(expr));
        if (c.containsKey("varset")) {
            o.append(",\"varset\":").append(q((String) c.get("varset")));
        } else {
            o.append(",\"vars\":").append(json(c.getOrDefault("vars", new ArrayList<>())));
        }
        if (c.containsKey("mode")) {
            o.append(",\"mode\":").append(q((String) c.get("mode")));
        }
        // 1) parse only
        Expression parsed;
        try {
            parsed = parser.parse(expr);
        } catch (Exception e) {
            o.append(",\"parse_error\":").append(q(e.getMessage())).append("}");
            return o.toString();
        } catch (Throwable t) {
            o.append(",\"parse_error\":").append(q(t.getClass().getName() + ": " + t.getMessage())).append("}");
            return o.toString();
        }
        o.append(",\"tree\":").append(q(tree(parsed)));
        List<String> ids = new ArrayList<>();
        for (Iterator<EObject> it = parsed.eAllContents(); it.hasNext();) {
            EObject x = it.next();
            if (x instanceof CharacterisedVariable) {
                try {
                    ids.add(SER.serialise((CharacterisedVariable) x));
                } catch (Exception e) {
                    ids.add("<" + e.getClass().getName() + ">");
                }
            }
        }
        if (parsed instanceof CharacterisedVariable) {
            try {
                ids.add(0, SER.serialise((CharacterisedVariable) parsed));
            } catch (Exception e) {
                ids.add(0, "<" + e.getClass().getName() + ">");
            }
        }
        o.append(",\"var_ids\":[");
        for (int i = 0; i < ids.size(); i++) {
            o.append(i > 0 ? "," : "").append(q(ids.get(i)));
        }
        o.append("]");
        // 2) the cache entry (parse + type inference + probfunction preparation)
        try {
            StoExCache.singleton().getEntry(expr);
        } catch (Throwable t) {
            o.append(",\"prepare_error\":").append(err(t)).append("}");
            return o.toString();
        }
        try {
            NonProbabilisticExpressionInferTypeVisitor tv = new NonProbabilisticExpressionInferTypeVisitor();
            tv.doSwitch(parsed);
            TypeEnum t = tv.getType(parsed);
            o.append(",\"type\":").append(t == null ? "null" : q(t.name()));
        } catch (Throwable t) {
            o.append(",\"type\":null");
        }
        // 3) evaluations
        SimulatedStackframe<Object> base = new SimulatedStackframe<>();
        SimulatedStackframe<Object> frame = new SimulatedStackframe<>();
        List<Object> vars = c.containsKey("varset") ? (List<Object>) VARSETS.get(c.get("varset"))
                : (List<Object>) c.getOrDefault("vars", new ArrayList<>());
        List<Object[]> proxies = new ArrayList<>();
        for (Object vo : vars) {
            Map<String, Object> v = (Map<String, Object>) vo;
            String id = (String) v.get("id");
            String t = (String) v.get("t");
            Object val = v.get("v");
            Object boxed;
            switch (t) {
            case "int":
                boxed = Integer.valueOf(((Number) val).intValue());
                break;
            case "double":
                boxed = Double.valueOf(num(val));
                break;
            case "bool":
                boxed = (Boolean) val;
                break;
            case "string":
                boxed = (String) val;
                break;
            case "proxy":
                proxies.add(new Object[] { id, val });
                continue;
            default:
                throw new IllegalArgumentException(t);
            }
            base.addValue(id, boxed);
            frame.addValue(id, boxed);
        }
        for (Object[] p : proxies) {
            frame.addValue((String) p[0], new EvaluationProxy((String) p[1], base.copyFrame()));
        }
        String m = (String) c.getOrDefault("mode", "EXCEPTION");
        VariableMode mode = m.equals("DEFAULT") ? VariableMode.RETURN_DEFAULT_ON_NOT_FOUND
                : m.equals("NULL") ? VariableMode.RETURN_NULL_ON_NOT_FOUND : VariableMode.EXCEPTION_ON_NOT_FOUND;
        List<Double> explicit = new ArrayList<>();
        for (Object u : (List<Object>) c.getOrDefault("uniforms", new ArrayList<>())) {
            explicit.add(num(u));
        }
        TAPE.reset(explicit, ((Number) c.getOrDefault("seed", 1L)).longValue());
        int evals = ((Number) c.getOrDefault("evals", 1L)).intValue();
        o.append(",\"evals\":[");
        for (int i = 0; i < evals; i++) {
            TAPE.drawn = new ArrayList<>();
            o.append(i > 0 ? "," : "").append("{");
            try {
                Object res = StackContext.evaluateStatic(expr, frame, mode);
                o.append("\"result\":").append(val(res));
            } catch (Throwable t) {
                o.append("\"error\":").append(err(t));
            }
            o.append(",\"draws\":[");
            for (int k = 0; k < TAPE.drawn.size(); k++) {
                o.append(k > 0 ? "," : "").append(q(hex(TAPE.drawn.get(k))));
            }
            o.append("]}");
        }
        o.append("]}");
        return o.toString();
    }

    static double num(Object o) {
        if (o instanceof String) {
            String s = (String) o;
            if (s.startsWith("0x")) {
                return Double.longBitsToDouble(Long.parseUnsignedLong(s.substring(2), 16));
            }
            return Double.parseDouble(s);
        }
        return ((Number) o).doubleValue();
    }

    static String hex(double d) {
        return String.format("%016x", Double.doubleToRawLongBits(d));
    }

    static String val(Object v) {
        if (v == null) {
            return "{\"t\":\"null\"}";
        }
        if (v instanceof Integer) {
            return "{\"t\":\"Integer\",\"v\":" + v + "}";
        }
        if (v instanceof Double) {
            return "{\"t\":\"Double\",\"v\":" + q(hex((Double) v)) + "}";
        }
        if (v instanceof Boolean) {
            return "{\"t\":\"Boolean\",\"v\":" + v + "}";
        }
        if (v instanceof String) {
            return "{\"t\":\"String\",\"v\":" + q((String) v) + "}";
        }
        return "{\"t\":" + q(v.getClass().getName()) + ",\"v\":" + q(String.valueOf(v)) + "}";
    }

    static String err(Throwable t) {
        List<String> chain = new ArrayList<>();
        Throwable root = t;
        chain.add(t.getClass().getName());
        while (root.getCause() != null && root.getCause() != root) {
            root = root.getCause();
            chain.add(root.getClass().getName());
        }
        StringBuilder b = new StringBuilder("{\"class\":").append(q(root.getClass().getName()));
        String msg = String.valueOf(root.getMessage());
        b.append(",\"message\":").append(q(msg.length() > 160 ? msg.substring(0, 160) : msg));
        b.append(",\"chain\":[");
        for (int i = 0; i < chain.size(); i++) {
            b.append(i > 0 ? "," : "").append(q(chain.get(i)));
        }
        return b.append("]}").toString();
    }

    // ------------------------------------------------------------------ tree dump (S-expression)

    static String tree(EObject e) {
        StringBuilder b = new StringBuilder();
        tree(e, b);
        return b.toString();
    }

    static void tree(EObject e, StringBuilder b) {
        if (e instanceof IfElseExpression) {
            IfElseExpression x = (IfElseExpression) e;
            b.append("(ifelse ");
            tree(x.getConditionExpression(), b);
            b.append(' ');
            tree(x.getIfExpression(), b);
            b.append(' ');
            tree(x.getElseExpression(), b);
            b.append(')');
        } else if (e instanceof BooleanOperatorExpression) {
            BooleanOperatorExpression x = (BooleanOperatorExpression) e;
            bin("bool " + x.getOperation().getName(), x.getLeft(), x.getRight(), b);
        } else if (e instanceof CompareExpression) {
            CompareExpression x = (CompareExpression) e;
            bin("cmp " + x.getOperation().getName(), x.getLeft(), x.getRight(), b);
        } else if (e instanceof TermExpression) {
            TermExpression x = (TermExpression) e;
            bin("term " + x.getOperation().getName(), x.getLeft(), x.getRight(), b);
        } else if (e instanceof ProductExpression) {
            ProductExpression x = (ProductExpression) e;
            bin("prod " + x.getOperation().getName(), x.getLeft(), x.getRight(), b);
        } else if (e instanceof PowerExpression) {
            PowerExpression x = (PowerExpression) e;
            bin("pow", x.getBase(), x.getExponent(), b);
        } else if (e instanceof NegativeExpression) {
            b.append("(neg ");
            tree(((NegativeExpression) e).getInner(), b);
            b.append(')');
        } else if (e instanceof NotExpression) {
            b.append("(not ");
            tree(((NotExpression) e).getInner(), b);
            b.append(')');
        } else if (e instanceof IntLiteral) {
            b.append("(int ").append(((IntLiteral) e).getValue()).append(')');
        } else if (e instanceof DoubleLiteral) {
            b.append("(double ").append(hex(((DoubleLiteral) e).getValue())).append(')');
        } else if (e instanceof StringLiteral) {
            b.append("(str ").append(q(((StringLiteral) e).getValue())).append(')');
        } else if (e instanceof BoolLiteral) {
            b.append("(boolean ").append(((BoolLiteral) e).isValue()).append(')');
        } else if (e instanceof FunctionLiteral) {
            FunctionLiteral f = (FunctionLiteral) e;
            b.append("(call ").append(f.getId());
            for (Expression p : f.getParameters_FunctionLiteral()) {
                b.append(' ');
                tree(p, b);
            }
            b.append(')');
        } else if (e instanceof CharacterisedVariable) {
            CharacterisedVariable v = (CharacterisedVariable) e;
            b.append("(var");
            AbstractNamedReference r = v.getId_Variable();
            while (r != null) {
                b.append(' ').append(q(r.getReferenceName()));
                r = r instanceof NamespaceReference ? ((NamespaceReference) r).getInnerReference_NamespaceReference()
                        : null;
            }
            b.append(' ').append(v.getCharacterisationType().getLiteral()).append(')');
        } else if (e instanceof Parenthesis) {
            b.append("(paren ");
            tree(((Parenthesis) e).getInnerExpression(), b);
            b.append(')');
        } else if (e instanceof ProbabilityFunctionLiteral) {
            EObject f = ((ProbabilityFunctionLiteral) e).getFunction_ProbabilityFunctionLiteral();
            if (f instanceof BoxedPDF) {
                b.append("(pdf");
                for (ContinuousSample s : ((BoxedPDF) f).getSamples()) {
                    b.append(" (").append(hex(s.getValue())).append(' ').append(hex(s.getProbability())).append(')');
                }
                b.append(')');
            } else {
                ProbabilityMassFunction<?> p = (ProbabilityMassFunction<?>) f;
                Object first = p.getSamples().get(0);
                String kind = first instanceof IntSample ? "intpmf"
                        : first instanceof DoubleSample ? "doublepmf"
                                : first instanceof StringSample ? "enumpmf" : first instanceof BoolSample ? "boolpmf" : "?";
                b.append('(').append(kind);
                if (p.isOrderedDomain()) {
                    b.append(" ordered");
                }
                for (Object so : p.getSamples()) {
                    Sample<?> s = (Sample<?>) so;
                    Object v = s.getValue();
                    String vs = v instanceof Double ? hex((Double) v) : v instanceof String ? q((String) v) : String.valueOf(v);
                    b.append(" (").append(vs).append(' ').append(hex(s.getProbability())).append(')');
                }
                b.append(')');
            }
        } else if (e instanceof VariableReference) {
            b.append("(ref ").append(q(((VariableReference) e).getReferenceName())).append(')');
        } else {
            b.append("(? ").append(e.eClass().getName()).append(')');
        }
    }

    static void bin(String head, EObject l, EObject r, StringBuilder b) {
        b.append('(').append(head).append(' ');
        tree(l, b);
        b.append(' ');
        tree(r, b);
        b.append(')');
    }

    // ------------------------------------------------------------------ minimal JSON

    @SuppressWarnings("unchecked")
    static String json(Object o) {
        if (o == null) {
            return "null";
        }
        if (o instanceof String) {
            return q((String) o);
        }
        if (o instanceof Map) {
            StringBuilder b = new StringBuilder("{");
            int i = 0;
            for (Map.Entry<String, Object> e : ((Map<String, Object>) o).entrySet()) {
                b.append(i++ > 0 ? "," : "").append(q(e.getKey())).append(':').append(json(e.getValue()));
            }
            return b.append('}').toString();
        }
        if (o instanceof List) {
            StringBuilder b = new StringBuilder("[");
            int i = 0;
            for (Object x : (List<Object>) o) {
                b.append(i++ > 0 ? "," : "").append(json(x));
            }
            return b.append(']').toString();
        }
        if (o instanceof Double) {
            return q("0x" + hex((Double) o));
        }
        return String.valueOf(o);
    }

    static String q(String s) {
        if (s == null) {
            return "null";
        }
        StringBuilder b = new StringBuilder("\"");
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
            case '"':
                b.append("\\\"");
                break;
            case '\\':
                b.append("\\\\");
                break;
            case '\n':
                b.append("\\n");
                break;
            case '\r':
                b.append("\\r");
                break;
            case '\t':
                b.append("\\t");
                break;
            default:
                if (c < 0x20 || c > 0x7e) {
                    b.append(String.format("\\u%04x", (int) c));
                } else {
                    b.append(c);
                }
            }
        }
        return b.append('"').toString();
    }

    /** Tiny JSON parser (objects, arrays, strings, numbers, true/false/null). */
    static final class Json {
        final String s;
        int i;

        Json(String s) {
            this.s = s;
        }

        Object value() {
            ws();
            char c = s.charAt(i);
            if (c == '{') {
                Map<String, Object> m = new LinkedHashMap<>();
                i++;
                ws();
                if (s.charAt(i) == '}') {
                    i++;
                    return m;
                }
                while (true) {
                    ws();
                    String k = str();
                    ws();
                    expect(':');
                    m.put(k, value());
                    ws();
                    if (s.charAt(i) == ',') {
                        i++;
                        continue;
                    }
                    expect('}');
                    return m;
                }
            }
            if (c == '[') {
                List<Object> l = new ArrayList<>();
                i++;
                ws();
                if (s.charAt(i) == ']') {
                    i++;
                    return l;
                }
                while (true) {
                    l.add(value());
                    ws();
                    if (s.charAt(i) == ',') {
                        i++;
                        continue;
                    }
                    expect(']');
                    return l;
                }
            }
            if (c == '"') {
                return str();
            }
            if (s.startsWith("true", i)) {
                i += 4;
                return Boolean.TRUE;
            }
            if (s.startsWith("false", i)) {
                i += 5;
                return Boolean.FALSE;
            }
            if (s.startsWith("null", i)) {
                i += 4;
                return null;
            }
            int st = i;
            while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) {
                i++;
            }
            String t = s.substring(st, i);
            if (t.contains(".") || t.contains("e") || t.contains("E")) {
                return Double.parseDouble(t);
            }
            return Long.parseLong(t);
        }

        String str() {
            expect('"');
            StringBuilder b = new StringBuilder();
            while (true) {
                char c = s.charAt(i++);
                if (c == '"') {
                    return b.toString();
                }
                if (c == '\\') {
                    char e = s.charAt(i++);
                    switch (e) {
                    case 'n':
                        b.append('\n');
                        break;
                    case 't':
                        b.append('\t');
                        break;
                    case 'r':
                        b.append('\r');
                        break;
                    case 'b':
                        b.append('\b');
                        break;
                    case 'f':
                        b.append('\f');
                        break;
                    case 'u':
                        b.append((char) Integer.parseInt(s.substring(i, i + 4), 16));
                        i += 4;
                        break;
                    default:
                        b.append(e);
                    }
                } else {
                    b.append(c);
                }
            }
        }

        void ws() {
            while (i < s.length() && Character.isWhitespace(s.charAt(i))) {
                i++;
            }
        }

        void expect(char c) {
            if (s.charAt(i) != c) {
                throw new IllegalArgumentException("expected " + c + " at " + i + " in " + s);
            }
            i++;
        }
    }
}
