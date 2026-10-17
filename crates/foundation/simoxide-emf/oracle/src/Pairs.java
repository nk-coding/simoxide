import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.emf.ecore.resource.*;
import org.eclipse.emf.ecore.util.EcoreUtil;

/**
 * Lenient comparisons of model pairs: each line of the list is
 *   <modelA> TAB <modelB> TAB <perturbation>
 * where the perturbation ("-" for none) is applied to B after loading:
 *   reverse <path> <feature> | setattr <path> <feature> <text> | delete <path>
 *   | retarget <path> <feature> <targetPath>
 * (paths in the canonical-dump form "/0/@f.1/@g"). Prints one verdict per line: EQUAL,
 * DIFFERENT or ERROR, using the Eclipse suite's comparer (SuiteCompare).
 */
public final class Pairs {
    private Pairs() {}

    static void run(Path list, Path out) throws IOException {
        StringBuilder sb = new StringBuilder();
        for (String line : Files.readAllLines(list)) {
            if (line.isBlank()) continue;
            String[] p = line.split("\t");
            String verdict;
            try {
                Resource a = EmfOracle.newResourceSet(p[0]).getResource(EmfOracle.fileUri(p[0]), true);
                ResourceSet rb = EmfOracle.newResourceSet(p[1]);
                Resource b = rb.getResource(EmfOracle.fileUri(p[1]), true);
                if (!p[2].equals("-")) perturb(b, p[2].split(" "));
                verdict = SuiteCompare.equal(a.getContents(), b.getContents()) ? "EQUAL" : "DIFFERENT";
            } catch (Exception e) {
                verdict = "ERROR";
            }
            sb.append(verdict).append('\n');
        }
        Files.write(out, sb.toString().getBytes(StandardCharsets.UTF_8));
    }

    static EObject find(Resource r, String path) {
        String[] segs = path.split("/");
        EObject cur = r.getContents().get(Integer.parseInt(segs[1]));
        for (int i = 2; i < segs.length; i++) {
            String s = segs[i].substring(1);
            int dot = s.lastIndexOf('.');
            if (dot >= 0 && s.substring(dot + 1).chars().allMatch(Character::isDigit)) {
                EStructuralFeature f = cur.eClass().getEStructuralFeature(s.substring(0, dot));
                cur = (EObject) ((List<?>) cur.eGet(f, false)).get(Integer.parseInt(s.substring(dot + 1)));
            } else {
                cur = (EObject) cur.eGet(cur.eClass().getEStructuralFeature(s), false);
            }
        }
        return cur;
    }

    @SuppressWarnings("unchecked")
    static void perturb(Resource r, String[] a) {
        EObject o = find(r, a[1]);
        switch (a[0]) {
            case "reverse": {
                EList<Object> l = (EList<Object>) o.eGet(o.eClass().getEStructuralFeature(a[2]), false);
                for (int i = 0; i < l.size(); i++) l.move(i, l.size() - 1);
                break;
            }
            case "setattr": o.eSet(o.eClass().getEStructuralFeature(a[2]), a[3]); break;
            case "delete": EcoreUtil.delete(o); break;
            case "retarget": o.eSet(o.eClass().getEStructuralFeature(a[2]), find(r, a[3])); break;
            default: throw new IllegalArgumentException(a[0]);
        }
    }
}
