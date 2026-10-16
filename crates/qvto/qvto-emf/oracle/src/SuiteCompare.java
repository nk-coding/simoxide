import java.util.List;

import org.eclipse.emf.ecore.EObject;
import org.eclipse.m2m.tests.qvt.oml.api.framework.comparator.TreeComparator;
import org.eclipse.m2m.tests.qvt.oml.api.framework.comparator.emf.EmfObjectComparatorTreeNode;

/** The Eclipse suite's model comparison (ModelTestData.compareWithExpected), root by root. */
public final class SuiteCompare {
    private SuiteCompare() {}

    public static boolean equal(List<EObject> expected, List<EObject> actual) {
        if (expected.size() != actual.size()) return false;
        for (int i = 0; i < expected.size(); i++) {
            int cost = new TreeComparator().compare(
                    new EmfObjectComparatorTreeNode(null, expected.get(i)),
                    new EmfObjectComparatorTreeNode(null, actual.get(i))).getCost();
            if (cost != 0) return false;
        }
        return true;
    }
}
