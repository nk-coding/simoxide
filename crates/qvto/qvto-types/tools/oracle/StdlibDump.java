// Dumps the standard library as Eclipse QVTo 3.11 builds it: the operations of classic OCL's
// predefined types (PredefinedType.oclOperations(), in order), the additional operations the
// QVTo standard library environment defines per owner type (QvtTypeResolverImpl's additional
// types, in definition order), and the operations owned by the QVTo library classes.
//
// Output (stdout), tab separated:
//   type  <kind>  <type>  [<supertypes...>]
//   op    <section>  <owner>  <name>  <params: name:type;...>  <return>  <flags>
// <section> is "ocl" (predefined operations of the owner), "qvto" (additional operations of the
// QVTo stdlib environment for the owner), "class" (EOperations of a QVTo library class).
// <flags>: comma separated: static, deprecated, deprecatedBy=<x>, unsupported=<reason>.
//
// Build and run: see run.sh. Licence: EPL-2.0.
import java.io.*;
import java.lang.reflect.Field;
import java.util.*;

import org.eclipse.emf.ecore.*;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtOperationalStdLibrary;
import org.eclipse.m2m.internal.qvt.oml.ast.env.QvtEnvironmentBase;
import org.eclipse.m2m.internal.qvt.oml.ast.parser.QvtOperationalParserUtil;
import org.eclipse.m2m.qvt.oml.ecore.ImperativeOCL.DictionaryType;
import org.eclipse.m2m.qvt.oml.ecore.ImperativeOCL.ListType;
import org.eclipse.ocl.types.CollectionType;
import org.eclipse.ocl.types.OCLStandardLibrary;
import org.eclipse.ocl.types.TupleType;
import org.eclipse.ocl.types.TypeType;
import org.eclipse.ocl.utilities.PredefinedType;

public class StdlibDump {
    static PrintStream out;

    public static void main(String[] args) throws Exception {
        out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
        QvtOperationalStdLibrary lib = QvtOperationalStdLibrary.INSTANCE;
        QvtEnvironmentBase env = (QvtEnvironmentBase) lib.getEnvironment();
        OCLStandardLibrary<EClassifier> ocl = env.getOCLStandardLibrary();

        List<EClassifier> predefined = new ArrayList<>(Arrays.asList(
            ocl.getOclAny(), ocl.getOclVoid(), ocl.getOclInvalid(), ocl.getBoolean(), ocl.getInteger(),
            ocl.getReal(), ocl.getString(), ocl.getUnlimitedNatural(), ocl.getOclType(),
            ocl.getOclElement(), ocl.getOclMessage(), ocl.getState(),
            ocl.getCollection(), ocl.getSet(), ocl.getOrderedSet(), ocl.getBag(), ocl.getSequence(),
            lib.getList(), lib.getDictionary()));
        // a TypeType instance
        predefined.add((EClassifier) env.getTypeResolver().resolveTypeType(ocl.getString()));
        for (EClassifier t : predefined) {
            out.println("type\tpredefined\t" + name(t) + "\t" + t.eClass().getName());
            if (t instanceof PredefinedType) {
                @SuppressWarnings("unchecked")
                List<EOperation> ops = ((PredefinedType<EOperation>) t).oclOperations();
                for (EOperation o : ops) op("ocl", t, o);
            }
        }

        // QVTo library classes and their own operations
        for (EClassifier c : lib.getStdLibModule().getEClassifiers()) {
            StringBuilder sup = new StringBuilder();
            if (c instanceof EClass) for (EClass s : ((EClass) c).getESuperTypes()) sup.append('\t').append(name(s));
            out.println("type\tstdlib\t" + name(c) + "\t" + c.eClass().getName() + sup
                + (c instanceof EClass && ((EClass) c).isAbstract() ? "\tabstract" : ""));
        }
        for (EClassifier c : lib.getStdLibModule().getEClassifiers()) {
            if (c instanceof EClass && !(c instanceof PredefinedType)) for (EOperation o : ((EClass) c).getEOperations()) op("class", c, o);
        }
        for (EOperation o : lib.getStdLibModule().getEOperations()) op("module", lib.getStdLibModule(), o);

        // additional operations of the stdlib environment, by owner
        Object resolver = env.getTypeResolver();
        Field f = resolver.getClass().getDeclaredField("fAdditionalTypes");
        f.setAccessible(true);
        @SuppressWarnings("unchecked")
        Set<EClassifier> owners = (Set<EClassifier>) f.get(resolver);
        java.lang.reflect.Method getDelegate = resolver.getClass().getDeclaredMethod("getDelegate");
        getDelegate.setAccessible(true);
        Object delegate = getDelegate.invoke(resolver);
        java.lang.reflect.Method addOps = null;
        for (Class<?> k = delegate.getClass(); k != null && addOps == null; k = k.getSuperclass())
            for (java.lang.reflect.Method m : k.getDeclaredMethods())
                if (m.getName().equals("getAdditionalOperations") && m.getParameterCount() == 1) { addOps = m; break; }
        addOps.setAccessible(true);
        for (EClassifier owner : owners) {
            out.println("owner\t" + name(owner));
            @SuppressWarnings("unchecked")
            List<EOperation> ops = (List<EOperation>) addOps.invoke(delegate, owner);
            for (EOperation o : ops) op("qvto", owner, o);
        }
    }

    static void op(String section, EClassifier owner, EOperation o) {
        StringBuilder ps = new StringBuilder();
        for (EParameter p : o.getEParameters()) {
            if (ps.length() > 0) ps.append(';');
            ps.append(p.getName()).append(':').append(name(p.getEType()));
        }
        List<String> flags = new ArrayList<>();
        if (QvtOperationalParserUtil.isStaticOperation(o)) flags.add("static");
        if (QvtOperationalParserUtil.isDeprecated(o)) {
            String by = QvtOperationalParserUtil.getDeprecatedBy(o);
            flags.add(by == null ? "deprecated" : "deprecatedBy=" + by);
        }
        if (QvtOperationalParserUtil.isUnsupported(o)) {
            String r = QvtOperationalParserUtil.getUnsupportedReason(o);
            flags.add("unsupported=" + (r == null ? "" : r.replace('\t', ' ')));
        }
        out.println("op\t" + section + "\t" + name(owner) + "\t" + o.getName() + "\t" + ps + "\t" + name(o.getEType()) + "\t" + String.join(",", flags));
    }

    static String name(EClassifier t) {
        if (t == null) return "null";
        if (t instanceof ListType) return "List(" + name(((ListType) t).getElementType()) + ")";
        if (t instanceof DictionaryType) return "Dict(" + name(((DictionaryType) t).getKeyType()) + "," + name(((DictionaryType) t).getElementType()) + ")";
        if (t instanceof CollectionType) {
            @SuppressWarnings("unchecked")
            CollectionType<EClassifier, EOperation> c = (CollectionType<EClassifier, EOperation>) t;
            return c.getKind().getName() + "(" + name(c.getElementType()) + ")";
        }
        if (t instanceof TypeType) {
            @SuppressWarnings("unchecked")
            TypeType<EClassifier, EOperation> tt = (TypeType<EClassifier, EOperation>) t;
            return "TypeType(" + name(tt.getReferredType()) + ")";
        }
        if (t instanceof TupleType) {
            StringBuilder b = new StringBuilder("Tuple(");
            boolean first = true;
            for (EStructuralFeature p : ((EClass) t).getEStructuralFeatures()) {
                if (!first) b.append(',');
                first = false;
                b.append(p.getName()).append(':').append(name(p.getEType()));
            }
            return b.append(')').toString();
        }
        EPackage p = t.getEPackage();
        String pn = p == null ? "?" : p.getName();
        return pn + "::" + t.getName();
    }
}
