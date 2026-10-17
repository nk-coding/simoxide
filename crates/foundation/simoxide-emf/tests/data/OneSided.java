import org.eclipse.emf.ecore.*;
import org.eclipse.emf.common.util.*;
import java.util.*;

public class OneSided {
	public static void main(String[] a) {
		EcoreFactory f = EcoreFactory.eINSTANCE;
		EPackage p = f.createEPackage(); p.setName("p"); p.setNsURI("p");
		EClass node = f.createEClass(); node.setName("Node"); p.getEClassifiers().add(node);
		EAttribute name = f.createEAttribute(); name.setName("name"); name.setEType(EcorePackage.Literals.ESTRING); node.getEStructuralFeatures().add(name);
		EReference kids = f.createEReference(); kids.setName("kids"); kids.setEType(node); kids.setContainment(true); kids.setUpperBound(-1); node.getEStructuralFeatures().add(kids);
		EReference parent = f.createEReference(); parent.setName("parent"); parent.setEType(node); node.getEStructuralFeatures().add(parent);
		kids.setEOpposite(parent); // one-sided: containment side
		EReference ops = f.createEReference(); ops.setName("ops"); ops.setEType(node); ops.setContainment(true); ops.setUpperBound(-1); node.getEStructuralFeatures().add(ops);
		EReference owner = f.createEReference(); owner.setName("owner"); owner.setEType(node); node.getEStructuralFeatures().add(owner);
		owner.setEOpposite(ops); // one-sided: non-containment side
		EReference peer = f.createEReference(); peer.setName("peer"); peer.setEType(node); node.getEStructuralFeatures().add(peer);
		EReference peer2 = f.createEReference(); peer2.setName("peer2"); peer2.setEType(node); node.getEStructuralFeatures().add(peer2);
		peer.setEOpposite(peer2);
		EObject A = p.getEFactoryInstance().create(node); A.eSet(name, "a");
		EObject B = p.getEFactoryInstance().create(node); B.eSet(name, "b");
		EObject C = p.getEFactoryInstance().create(node); C.eSet(name, "c");
		@SuppressWarnings("unchecked") EList<EObject> ak = (EList<EObject>) A.eGet(kids);
		ak.add(B);
		System.out.println("1 B.parent=" + n(B.eGet(parent), name) + " B.container=" + n(B.eContainer(), name) + " feat=" + (B.eContainingFeature()==null?null:B.eContainingFeature().getName()) + " B.isSet(parent)=" + B.eIsSet(parent));
		B.eSet(parent, C);
		System.out.println("2 B.parent=" + n(B.eGet(parent), name) + " A.kids=" + ak.size() + " C.kids=" + ((List<?>)C.eGet(kids)).size() + " B.container=" + n(B.eContainer(), name));
		ak.remove(B);
		System.out.println("3 B.parent=" + n(B.eGet(parent), name) + " B.container=" + n(B.eContainer(), name));
		@SuppressWarnings("unchecked") EList<EObject> aops = (EList<EObject>) A.eGet(ops);
		aops.add(B);
		System.out.println("4 B.owner=" + n(B.eGet(owner), name) + " B.container=" + n(B.eContainer(), name) + " feat=" + (B.eContainingFeature()==null?null:B.eContainingFeature().getName()));
		B.eSet(owner, C);
		System.out.println("5 B.owner=" + n(B.eGet(owner), name) + " A.ops=" + aops.size() + " C.ops=" + ((List<?>)C.eGet(ops)).size() + " B.container=" + n(B.eContainer(), name));
		A.eSet(peer, B);
		System.out.println("6 B.peer2=" + n(B.eGet(peer2), name) + " A.peer2=" + n(A.eGet(peer2), name));
		B.eSet(peer2, C);
		System.out.println("7 A.peer=" + n(A.eGet(peer), name) + " C.peer=" + n(C.eGet(peer), name));
		A.eSet(peer, C);
		System.out.println("8 B.peer2=" + n(B.eGet(peer2), name) + " C.peer2=" + n(C.eGet(peer2), name));
	}
	static String n(Object o, EAttribute name) { return o == null ? "null" : String.valueOf(((EObject)o).eGet(name)); }
}
