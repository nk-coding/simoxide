package org.palladiosimulator.architecturaltemplates.catalog.black;

import org.eclipse.emf.ecore.EObject;
import org.eclipse.m2m.qvt.oml.blackbox.java.Module;

/**
 * Stub of Palladio's stereotype blackbox (architecturaltemplates.catalog, which needs EMF
 * Profiles and the generated PCM classes). The SPD semantic transformations import it but call
 * none of its operations; the stub lets the import resolve. Every operation fails if called.
 */
@Module(packageURIs = { "http://www.eclipse.org/emf/2002/Ecore" })
public class ProfilesLibrary {
	public ProfilesLibrary() {
	}

	private static RuntimeException stub() {
		return new UnsupportedOperationException("qvto-rs reference runner: ProfilesLibrary stub");
	}

	public static boolean isProfileApplied(EObject eObject, String profileName) {
		throw stub();
	}

	public static void applyProfile(EObject eObject, String profileName) {
		throw stub();
	}

	public static boolean hasAppliedStereotype(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static void applyStereotype(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static void removeStereotypeApplications(EObject entity, String stereotypeName) {
		throw stub();
	}

	public static Object getTaggedValue(EObject entity, String taggedValueName, String stereotypeName) {
		throw stub();
	}

	public static void setTaggedValue(EObject entity, Object value, String taggedValueName, String stereotypeName) {
		throw stub();
	}
}
