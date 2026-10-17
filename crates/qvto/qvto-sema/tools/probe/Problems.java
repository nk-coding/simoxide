// Probe of Eclipse QVTo for qvto-rs (qvto-sema/tools/probe/probe.sh). EPL-2.0.
import org.eclipse.emf.common.util.*;
import org.eclipse.m2m.internal.qvt.oml.InternalTransformationExecutor;
import org.eclipse.m2m.internal.qvt.oml.compiler.CompiledUnit;
import org.eclipse.m2m.internal.qvt.oml.ast.parser.QvtOperationalParserUtil;
import org.eclipse.m2m.internal.qvt.oml.QvtMessage;
import org.eclipse.core.runtime.NullProgressMonitor;

/** Prints the compile problems of a unit: line offset length message. */
public class Problems {
	public static void main(String[] a) throws Exception {
		for (String f : a) {
			URI uri = URI.createFileURI(new java.io.File(f).getAbsolutePath());
			InternalTransformationExecutor k = new InternalTransformationExecutor(uri);
			k.loadTransformation(new NullProgressMonitor());
			CompiledUnit u = k.getUnit();
			System.out.println("== " + f);
			if (u == null) { System.out.println("no unit"); continue; }
			for (QvtMessage m : u.getProblems()) {
				System.out.println((m.getSeverity() == QvtMessage.SEVERITY_ERROR ? "E " : "W ") + m.getLineNum() + " " + m.getOffset() + " " + m.getLength() + " " + m.getMessage());
			}
		}
	}
}
