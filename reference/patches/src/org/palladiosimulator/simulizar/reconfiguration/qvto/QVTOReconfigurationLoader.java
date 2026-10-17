package org.palladiosimulator.simulizar.reconfiguration.qvto;

import javax.inject.Inject;

import org.eclipse.emf.common.util.URI;
import org.palladiosimulator.simulizar.core.runconfig.SimuLizarWorkflowConfiguration;
import org.palladiosimulator.simulizar.di.base.scopes.RuntimeExtensionScope;
import org.palladiosimulator.simulizar.reconfiguration.AbstractReconfigurationLoader;
import org.palladiosimulator.simulizar.reconfiguration.qvto.util.ModelTransformationCache;

@RuntimeExtensionScope
public class QVTOReconfigurationLoader extends AbstractReconfigurationLoader {

	private static final String QVTO_FILE_EXTENSION = ".qvto";
	
	@Inject
	public QVTOReconfigurationLoader() {
    }

	// REFSIM (patch P9): the rules folder is a file URI; list its .qvto files in file-name order.
	// Stock: FileHelper.getURIs (needs Eclipse's FileLocator, which a flat classpath lacks) and
	// one ModelTransformationCache whose HashMap<URI, ...> decides the rule order (the URIs' hash
	// order, which depends on where the folder lies). SimOxide runs the rules in file-name order.
	@Override
	public void load(SimuLizarWorkflowConfiguration configuration) {
		String folder = configuration.getReconfigurationRulesFolder();
		if (folder == null || folder.isEmpty()) {
			return;
		}
		java.io.File dir = new java.io.File(URI.createURI(folder).toFileString());
		java.io.File[] files = dir.listFiles((d, n) -> n.endsWith(QVTO_FILE_EXTENSION));
		if (files == null || files.length == 0) {
			return;
		}
		java.util.Arrays.sort(files, java.util.Comparator.comparing(java.io.File::getName));
		URI[] uris = new URI[files.length];
		for (int i = 0; i < files.length; i++) {
			uris[i] = URI.createURI(folder).appendSegment(files[i].getName());
		}
		setTransformations(uris);
	}

	@Override
	protected String getTransformationFileExtension() {
		return QVTO_FILE_EXTENSION;
	}

	@Override
	protected void setTransformations(URI[] transURIs) {
		// REFSIM (patch P9): one cache per rule keeps the given order
		for (URI u : transURIs) {
			new ModelTransformationCache(u).getAll().forEach(t -> this.transformations.add(t));
		}
	}
}
