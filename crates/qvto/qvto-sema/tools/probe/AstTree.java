// Probe of Eclipse QVTo for qvto-rs (crates/qvto-sema/tools/probe/probe.sh). EPL-2.0.
import org.eclipse.emf.common.util.*;
import org.eclipse.emf.ecore.*;
import org.eclipse.m2m.internal.qvt.oml.InternalTransformationExecutor;
import org.eclipse.m2m.internal.qvt.oml.compiler.CompiledUnit;
import org.eclipse.m2m.internal.qvt.oml.expressions.Module;
import org.eclipse.ocl.utilities.ASTNode;
import org.eclipse.core.runtime.NullProgressMonitor;

/** Prints the AST of a compiled unit's modules (operations and their contents) as an indented tree. */
public class AstTree {
	static void dump(EObject o, String ind) {
		int s = o instanceof ASTNode ? ((ASTNode) o).getStartPosition() : -1;
		int e = o instanceof ASTNode ? ((ASTNode) o).getEndPosition() : -1;
		String name = o instanceof ENamedElement ? " '" + ((ENamedElement) o).getName() + "'" : "";
		String ty = "";
		if (o instanceof ETypedElement && ((ETypedElement) o).getEType() != null) ty = " : " + ((ETypedElement) o).getEType().getName();
		System.out.println(ind + o.eClass().getName() + name + ty + " [" + s + "," + e + "]");
		for (EObject c : o.eContents()) dump(c, ind + "  ");
	}
	public static void main(String[] a) throws Exception {
		URI uri = URI.createFileURI(new java.io.File(a[0]).getAbsolutePath());
		InternalTransformationExecutor k = new InternalTransformationExecutor(uri);
		k.loadTransformation(new NullProgressMonitor());
		CompiledUnit u = k.getUnit();
		for (Module m : u.getModules()) { System.out.println("module " + m.getName()); for (EOperation op : m.getEOperations()) dump(op, ""); }
	}
}
