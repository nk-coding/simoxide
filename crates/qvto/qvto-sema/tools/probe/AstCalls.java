// Probe of Eclipse QVTo for qvto-rs (crates/qvto-sema/tools/probe/probe.sh). EPL-2.0.
import java.util.*;
import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.m2m.internal.qvt.oml.InternalTransformationExecutor;
import org.eclipse.m2m.internal.qvt.oml.compiler.CompiledUnit;
import org.eclipse.m2m.internal.qvt.oml.expressions.Module;
import org.eclipse.ocl.utilities.ASTNode;
import org.eclipse.core.runtime.NullProgressMonitor;

/** Prints the referred operation of every operation call of a compiled transformation. */
public class AstCalls {
	static String tn(EClassifier c) {
		if (c == null) return "null";
		String n = c.getName();
		EObject cont = c.eContainer();
		return n + (cont instanceof ENamedElement ? "@" + ((ENamedElement) cont).getName() : "");
	}
	public static void main(String[] a) throws Exception {
		URI uri = URI.createFileURI(new java.io.File(a[0]).getAbsolutePath());
		InternalTransformationExecutor k = new InternalTransformationExecutor(uri);
		Diagnostic d = k.loadTransformation(new NullProgressMonitor());
		System.out.println("load " + d.getSeverity() + " " + d.getMessage());
		for (Diagnostic c : d.getChildren()) System.out.println("  " + c.getMessage());
		CompiledUnit u = k.getUnit();
		for (Module m : u.getModules()) {
			TreeIterator<EObject> it = m.eAllContents();
			while (it.hasNext()) {
				EObject o = it.next();
				if (o instanceof org.eclipse.ocl.expressions.OperationCallExp) {
					org.eclipse.ocl.expressions.OperationCallExp<?, ?> call = (org.eclipse.ocl.expressions.OperationCallExp<?, ?>) o;
					EOperation op = (EOperation) call.getReferredOperation();
					StringBuilder sb = new StringBuilder();
					int pos = (o instanceof ASTNode) ? ((ASTNode) o).getStartPosition() : -1;
					sb.append(pos).append(' ');
					if (op == null) { sb.append("<null>"); System.out.println(sb); continue; }
					sb.append(op.getName()).append('(');
					for (EParameter p : op.getEParameters()) sb.append(p.getName()).append(':').append(tn(p.getEType())).append(',');
					sb.append(") : ").append(tn(op.getEType()));
					sb.append(" owner ").append(op.eContainer() instanceof EClassifier ? tn((EClassifier) op.eContainer()) : String.valueOf(op.eContainer()));
					sb.append(" type ").append(tn(((org.eclipse.ocl.expressions.OCLExpression<EClassifier>) (Object) call).getType()));
					System.out.println(sb);
				}
			}
		}
	}
}
