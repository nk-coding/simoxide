// Parse-only oracle: runs Eclipse QVTo's LPG lexer and parser (no semantic analysis) on each
// file given on stdin (one path per line) and prints its syntax problems.
//
// Output, one line per problem, tab separated:
//   path  E|W  offset  length  line  message
// and "path  OK" for a file without problems; "path  E  0  0  1  CRASH <exception>" when the
// parser throws. Offsets are Java char offsets (UTF-16) into the
// text after Eclipse's lone-CR correction. With -time, prints the total parse time to stderr.
//
// Build and run: see tools/oracle/run.sh. Licence: EPL-2.0.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.EPackage;
import org.eclipse.emf.ecore.impl.EPackageRegistryImpl;
import org.eclipse.m2m.internal.qvt.oml.QvtMessage;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtOperationalEnvFactory;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtOperationalFileEnv;
import org.eclipse.m2m.internal.qvt.oml.ast.parser.QvtOperationalParser;

public class ParseOracle {
    public static void main(String[] args) throws Exception {
        boolean time = args.length > 0 && args[0].equals("-time");
        int repeat = args.length > 1 ? Integer.parseInt(args[1]) : 1;
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        List<String> paths = new ArrayList<>();
        for (String l; (l = in.readLine()) != null;) if (!l.isBlank()) paths.add(l.trim());
        List<String> texts = new ArrayList<>();
        long bytes = 0;
        for (String p : paths) {
            byte[] b = Files.readAllBytes(Paths.get(p));
            bytes += b.length;
            texts.add(new String(b, StandardCharsets.UTF_8));
        }
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
        EPackage.Registry reg = new EPackageRegistryImpl();
        long best = Long.MAX_VALUE;
        for (int r = 0; r < repeat; r++) {
            long t0 = System.nanoTime();
            for (int i = 0; i < paths.size(); i++) {
                String p = paths.get(i);
                QvtOperationalFileEnv env = new QvtOperationalEnvFactory(reg).createEnvironment(URI.createFileURI(p));
                try {
                    new QvtOperationalParser().parse(new StringReader(texts.get(i)), new File(p).getName(), env);
                } catch (RuntimeException e) {
                    // The LPG parser's recovery actions can throw (e.g. ClassCastException):
                    // the compiler then fails, so report it as an error at offset 0.
                    if (r == 0) out.println(p + "\tE\t0\t0\t1\tCRASH " + e.getClass().getName());
                    continue;
                }
                if (r > 0) continue;
                List<QvtMessage> msgs = env.getAllProblemMessages();
                if (msgs.isEmpty()) { out.println(p + "\tOK"); continue; }
                for (QvtMessage m : msgs) {
                    String sev = m.getSeverity() == QvtMessage.SEVERITY_ERROR ? "E" : "W";
                    String msg = m.getMessage() == null ? "" : m.getMessage().replace('\t', ' ').replace('\n', ' ').replace('\r', ' ');
                    out.println(p + "\t" + sev + "\t" + m.getOffset() + "\t" + m.getLength() + "\t" + m.getLineNum() + "\t" + msg);
                }
            }
            long dt = System.nanoTime() - t0;
            best = Math.min(best, dt);
        }
        if (time) System.err.printf("files=%d bytes=%d best_ms=%.1f MB/s=%.2f%n", paths.size(), bytes, best / 1e6, bytes / (best / 1e9) / 1e6);
    }
}
