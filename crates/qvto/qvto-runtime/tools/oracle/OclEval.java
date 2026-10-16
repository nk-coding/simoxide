// Evaluates classic Eclipse OCL expressions as QVTo configures the parser (long integer
// literals on), without a context object. Results use Oracle's value encoding.
import org.eclipse.emf.ecore.EClassifier;
import org.eclipse.emf.ecore.EcorePackage;
import org.eclipse.ocl.ecore.Constraint;
import org.eclipse.ocl.ecore.EcoreEnvironmentFactory;
import org.eclipse.ocl.ecore.OCL;
import org.eclipse.ocl.expressions.OCLExpression;
import org.eclipse.ocl.helper.OCLHelper;
import org.eclipse.ocl.options.ParsingOptions;

final class OclEval {
    private static OCL ocl;
    private static OCLHelper<EClassifier, ?, ?, Constraint> helper;

    static String eval(String expr) {
        if (ocl == null) {
            ocl = OCL.newInstance(EcoreEnvironmentFactory.INSTANCE);
            ParsingOptions.setOption(ocl.getEnvironment(), ParsingOptions.USE_LONG_INTEGERS, true);
            helper = ocl.createOCLHelper();
            helper.setContext(EcorePackage.Literals.EPACKAGE);
        }
        OCLExpression<EClassifier> q;
        try {
            q = helper.createQuery(expr);
        } catch (Exception e) {
            return "PARSE";
        }
        Object r = ocl.evaluate(EcorePackage.eINSTANCE, q);
        if (ocl.isInvalid(r)) return "X";
        return Oracle.enc(r);
    }
}
